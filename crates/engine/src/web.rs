//! Local web UI for playing against the engine.
//!
//! A minimal HTTP/1.1 server on localhost (no dependencies, no external assets):
//! the page is embedded in the binary. Game state lives on the server and the
//! engine validates every move, so the board in the browser cannot drift out of
//! sync with the rules.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::AtomicBool,
    time::Instant,
};

use movegen::{Board, Color, Move, MoveList, START_FEN};

use crate::{
    search::{self, Limits},
    tt::TranspositionTable,
};

const INDEX: &str = include_str!("../assets/index.html");
const APP_JS: &str = include_str!("../assets/app.js");
const STYLE_CSS: &str = include_str!("../assets/style.css");

/// Strength presets: node budget per move, plus a hard time cap. Only `full`
/// has been calibrated by matches; the others are rough guesses.
struct Level {
    id: &'static str,
    label: &'static str,
    nodes: u64,
    movetime_ms: u64,
    /// Play a uniformly random legal move instead of searching.
    random: bool,
}

const LEVELS: [Level; 6] = [
    Level { id: "full", label: "полная сила", nodes: 20_000_000, movetime_ms: 2_000, random: false },
    Level { id: "hard", label: "сложно", nodes: 300_000, movetime_ms: 400, random: false },
    Level { id: "medium", label: "средне", nodes: 40_000, movetime_ms: 100, random: false },
    Level { id: "easy", label: "легко", nodes: 4_000, movetime_ms: 30, random: false },
    Level { id: "beginner", label: "новичок", nodes: 600, movetime_ms: 10, random: false },
    Level { id: "random", label: "случайные ходы", nodes: 0, movetime_ms: 0, random: true },
];

/// xorshift: only used to pick random moves for the weakest level.
struct Rng(u64);

impl Rng {
    fn new() -> Rng {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        Rng(nanos | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

pub fn run(port: u16) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    println!("chess-ai web UI: http://127.0.0.1:{port}/");
    println!("Ctrl+C останавливает сервер.");
    let mut game = Game::new();
    let mut last_seen: Option<Instant> = None;
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        // A fresh tab means a fresh game.
        if last_seen.is_some_and(|t| t.elapsed().as_secs() > 3600) {
            game = Game::new();
        }
        last_seen = Some(Instant::now());
        if let Err(e) = handle(&mut stream, &mut game) {
            eprintln!("запрос не обработан: {e}");
        }
    }
    Ok(())
}

fn handle(stream: &mut TcpStream, game: &mut Game) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        if header.trim().is_empty() {
            break;
        }
        if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    let body = String::from_utf8_lossy(&body).into_owned();

    let response = match (method.as_str(), path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => text("text/html; charset=utf-8", INDEX),
        ("GET", "/app.js") => text("application/javascript; charset=utf-8", APP_JS),
        ("GET", "/style.css") => text("text/css; charset=utf-8", STYLE_CSS),
        ("GET", "/api/state") => json(game.state(None)),
        ("POST", "/api/new") => {
            let level = field(&body, "level").unwrap_or_else(|| "full".to_string());
            *game = Game::new();
            game.level = level;
            json(game.state(None))
        }
        ("POST", "/api/level") => {
            if let Some(level) = field(&body, "level") {
                game.level = level;
            }
            json(game.state(None))
        }
        ("POST", "/api/hint") => {
            let reply = game.hint();
            json(match reply {
                Some(r) => format!(
                    "{{\"move\":{},\"score\":{},\"mate\":{},\"depth\":{},\"nodes\":{},\"ms\":{}}}",
                    quote(&r.mv),
                    r.score,
                    search::is_mate_score(r.score),
                    r.depth,
                    r.nodes,
                    r.ms
                ),
                None => "null".to_string(),
            })
        }
        ("POST", "/api/move") => match field(&body, "from").zip(field(&body, "to")) {
            Some((from, to)) => {
                let promo = field(&body, "promo").unwrap_or_default();
                let uci = format!("{from}{to}{promo}");
                match game.play(&uci) {
                    Ok(()) => json(game.state(game.engine_reply.as_ref())),
                    Err(e) => json(error(&e)),
                }
            }
            None => json(error("нужны поля from и to")),
        },
        ("POST", "/api/undo") => {
            game.undo();
            json(game.state(None))
        }
        _ => {
            let mut out = String::from("HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n");
            out.push_str("не найдено");
            out
        }
    };
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn text(kind: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn json(body: String) -> String {
    text("application/json; charset=utf-8", &body)
}

/// `a=1&b=2` form body.
fn field(body: &str, name: &str) -> Option<String> {
    body.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.to_string())
    })
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn error(message: &str) -> String {
    format!("{{\"error\":{}}}", quote(message))
}

struct EngineReply {
    mv: String,
    score: i32,
    depth: u32,
    nodes: u64,
    ms: u128,
}

#[derive(PartialEq, Eq)]
enum Outcome {
    Playing,
    Checkmate(Color),
    Stalemate,
    Draw(&'static str),
}

struct Game {
    board: Board,
    /// Hashes of positions before `board`, for repetition detection.
    hashes: Vec<u64>,
    /// Moves played in this game, in UCI text.
    moves: Vec<String>,
    tt: TranspositionTable,
    level: String,
    engine_reply: Option<EngineReply>,
    rng: Rng,
}

impl Game {
    fn new() -> Game {
        Game {
            board: Board::from_fen(START_FEN).expect("start position is valid"),
            hashes: Vec::new(),
            moves: Vec::new(),
            tt: TranspositionTable::new(16),
            level: LEVELS[0].id.to_string(),
            engine_reply: None,
            rng: Rng::new(),
        }
    }

    fn level(&self) -> &Level {
        LEVELS.iter().find(|l| l.id == self.level).unwrap_or(&LEVELS[0])
    }

    /// Applies the player's move and answers with the engine's.
    fn play(&mut self, uci: &str) -> Result<(), String> {
        if self.outcome() != Outcome::Playing {
            return Err("партия закончена".into());
        }
        let mv = self.board.find_uci_move(uci).ok_or_else(|| format!("недопустимый ход {uci}"))?;
        self.push(mv);
        self.engine_reply = None;
        if self.outcome() == Outcome::Playing {
            let reply = self.search();
            if let Some(reply) = reply {
                if let Some(mv) = self.board.find_uci_move(&reply.mv) {
                    self.push(mv);
                }
                self.engine_reply = Some(reply);
            }
        }
        Ok(())
    }

    fn push(&mut self, mv: Move) {
        self.hashes.push(self.board.hash());
        self.moves.push(mv.to_string());
        self.board.make_move(mv);
    }

    /// Takes back the engine's reply and the player's move.
    fn undo(&mut self) {
        let back = if self.engine_reply.is_some() { 2 } else { 1 };
        if self.moves.len() < back {
            return;
        }
        let mut board = Board::from_fen(START_FEN).expect("start position is valid");
        let mut hashes = Vec::new();
        for text in &self.moves[..self.moves.len() - back] {
            hashes.push(board.hash());
            let mv = board.find_uci_move(text).expect("recorded move is legal");
            board.make_move(mv);
        }
        self.board = board;
        self.hashes = hashes;
        self.moves.truncate(self.moves.len() - back);
        self.engine_reply = None;
    }

    fn search(&mut self) -> Option<EngineReply> {
        let level = self.level();
        if level.random {
            let mut moves = MoveList::new();
            self.board.generate_moves(&mut moves);
            if moves.is_empty() {
                return None;
            }
            let pick = (self.rng.next() % moves.len() as u64) as usize;
            return Some(EngineReply {
                mv: moves[pick].to_string(),
                score: 0,
                depth: 0,
                nodes: moves.len() as u64,
                ms: 0,
            });
        }
        self.run_search(level.nodes, level.movetime_ms)
    }

    /// Best move for the side to move, without playing it.
    fn hint(&mut self) -> Option<EngineReply> {
        self.run_search(200_000, 300)
    }

    fn run_search(&mut self, nodes: u64, movetime_ms: u64) -> Option<EngineReply> {
        let stop = AtomicBool::new(false);
        let limits = Limits { nodes: Some(nodes), movetime: Some(movetime_ms), ..Limits::default() };
        let start = Instant::now();
        let result = search::search(&self.board, self.hashes.clone(), &limits, &stop, &mut self.tt, false);
        if result.best.is_null() {
            return None;
        }
        Some(EngineReply {
            mv: result.best.to_string(),
            score: result.score,
            depth: result.depth,
            nodes: result.nodes,
            ms: start.elapsed().as_millis(),
        })
    }

    fn outcome(&self) -> Outcome {
        let mut moves = MoveList::new();
        self.board.generate_moves(&mut moves);
        if moves.is_empty() {
            return if self.board.in_check() {
                Outcome::Checkmate(!self.board.side_to_move())
            } else {
                Outcome::Stalemate
            };
        }
        if self.board.halfmove_clock() >= 100 {
            return Outcome::Draw("правило 50 ходов");
        }
        let window = (self.board.halfmove_clock() as usize).min(self.hashes.len());
        let repeats =
            (2..=window).step_by(2).filter(|&back| self.hashes[self.hashes.len() - back] == self.board.hash()).count();
        if repeats >= 2 {
            return Outcome::Draw("троекратное повторение");
        }
        Outcome::Playing
    }

    fn status(&self) -> String {
        match self.outcome() {
            Outcome::Playing => {
                if self.board.side_to_move() == Color::White {
                    "your_move".to_string()
                } else {
                    "engine_move".to_string()
                }
            }
            Outcome::Checkmate(winner) => format!("mate_{}", if winner == Color::White { "white" } else { "black" }),
            Outcome::Stalemate => "stalemate".to_string(),
            Outcome::Draw(reason) => format!("draw: {reason}"),
        }
    }

    fn state(&self, reply: Option<&EngineReply>) -> String {
        let mut moves = MoveList::new();
        self.board.generate_moves(&mut moves);
        let legal: Vec<String> = moves.iter().map(|m| quote(&m.to_string())).collect();
        let mut out = String::with_capacity(512);
        out.push_str("{\"fen\":");
        out.push_str(&quote(&self.board.to_fen()));
        out.push_str(",\"legal\":[");
        out.push_str(&legal.join(","));
        out.push_str("],\"check\":");
        out.push_str(if self.board.in_check() { "true" } else { "false" });
        out.push_str(",\"status\":");
        out.push_str(&quote(&self.status()));
        out.push_str(",\"moves\":[");
        out.push_str(
            &self.moves.iter().map(|m| quote(m)).collect::<Vec<_>>().join(","),
        );
        out.push_str("],\"level\":");
        out.push_str(&quote(&self.level));
        out.push_str(",\"levels\":[");
        out.push_str(
            &LEVELS
                .iter()
                .map(|l| format!("{{\"id\":{},\"label\":{}}}", quote(l.id), quote(l.label)))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push_str("],\"info\":");
        match reply {
            Some(r) => out.push_str(&format!(
                "{{\"move\":{},\"score\":{},\"mate\":{},\"depth\":{},\"nodes\":{},\"ms\":{}}}",
                quote(&r.mv),
                r.score,
                search::is_mate_score(r.score),
                r.depth,
                r.nodes,
                r.ms
            )),
            None => out.push_str("null"),
        }
        out.push('}');
        out
    }
}

/// Depth-limited helper used by the tests below.
#[cfg(test)]
mod tests {
    use super::*;

    fn play(game: &mut Game, uci: &str) {
        game.play(uci).expect("move is legal");
    }

    #[test]
    fn plays_a_full_move_pair() {
        let mut game = Game::new();
        game.level = "easy".into();
        play(&mut game, "e2e4");
        assert_eq!(game.moves.first().map(String::as_str), Some("e2e4"));
        assert_eq!(game.moves.len(), 2, "движок должен ответить");
        assert_eq!(game.board.side_to_move(), Color::White);
    }

    #[test]
    fn rejects_illegal_moves_and_terminal_positions() {
        let mut game = Game::new();
        game.level = "easy".into();
        assert!(game.play("e2e5").is_err());
        // Fool's mate: the game must end and no further move is accepted.
        for mv in ["f2f3", "e7e5", "g2g4", "d8h4"] {
            game.push(game.board.find_uci_move(mv).unwrap());
        }
        assert_eq!(game.status(), "mate_black");
        assert!(game.play("e1f2").is_err());
    }

    #[test]
    fn undo_restores_the_previous_position() {
        let mut game = Game::new();
        game.level = "easy".into();
        let start = game.board.to_fen();
        play(&mut game, "d2d4");
        assert_ne!(game.board.to_fen(), start);
        game.undo();
        assert_eq!(game.board.to_fen(), start);
        assert!(game.moves.is_empty());
    }

    #[test]
    fn state_is_valid_json_shape() {
        let mut game = Game::new();
        game.level = "easy".into();
        play(&mut game, "e2e4");
        let state = game.state(game.engine_reply.as_ref());
        assert!(state.starts_with("{\"fen\":\""));
        assert!(state.contains("\"legal\":["));
        assert!(state.contains("\"status\":\"your_move\""));
        assert!(state.contains("\"info\":{\"move\":\""));
    }
}

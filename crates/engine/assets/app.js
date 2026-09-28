// Board rendering and interaction. All chess rules live on the server:
// this file only draws the position it is given and reports clicks.

const GLYPH = {
  K: "\u2654", Q: "\u2655", R: "\u2656", B: "\u2657", N: "\u2658", P: "\u2659",
  k: "\u265A", q: "\u265B", r: "\u265C", b: "\u265D", n: "\u265E", p: "\u265F",
};
const FILES = "abcdefgh";

const board = document.getElementById("board");
const statusEl = document.getElementById("status");
const infoEl = document.getElementById("info");
const movesEl = document.getElementById("moves");
const levelEl = document.getElementById("level");
const promoEl = document.getElementById("promo");
const newBtn = document.getElementById("new");
const undoBtn = document.getElementById("undo");
const hintBtn = document.getElementById("hintBtn");
const hintEl = document.getElementById("hintText");

let state = { fen: "", legal: [], moves: [], status: "", info: null, check: false };
let selected = null;
let pending = null; // {from, to} waiting for a promotion choice
let hint = null; // {from, to, text} shown until the next move
let busy = false;

// fenPlacement(fen) -> { square: pieceChar }
function fenPieces(fen) {
  const out = {};
  const [placement] = fen.split(" ");
  let rank = 7;
  let file = 0;
  for (const ch of placement) {
    if (ch === "/") {
      rank -= 1;
      file = 0;
    } else if (ch >= "1" && ch <= "8") {
      file += Number(ch);
    } else {
      out[FILES[file] + (rank + 1)] = ch;
      file += 1;
    }
  }
  return out;
}

function legalFrom(square) {
  return state.legal.filter((m) => m.slice(0, 2) === square);
}

function render() {
  const pieces = fenPieces(state.fen);
  const last = state.moves.slice(-1)[0] || "";
  const checkSquare = state.check ? findKing(pieces, state.fen.split(" ")[1] === "w") : null;
  const targets = selected ? new Set(legalFrom(selected).map((m) => m.slice(2, 4))) : new Set();
  const hintSquares = hint ? new Set([hint.from, hint.to]) : new Set();

  board.textContent = "";
  for (let rank = 8; rank >= 1; rank -= 1) {
    for (let file = 0; file < 8; file += 1) {
      const square = FILES[file] + rank;
      const sq = document.createElement("div");
      sq.className = "sq " + ((file + rank) % 2 === 0 ? "dark" : "light");
      sq.dataset.square = square;
      if (square === selected) sq.classList.add("sel");
      if (hintSquares.has(square)) sq.classList.add("hint");
      if (last.includes(square)) sq.classList.add("last");
      if (square === checkSquare) sq.classList.add("check");

      const piece = pieces[square];
      if (piece) {
        const span = document.createElement("span");
        span.className = "piece " + (piece === piece.toUpperCase() ? "white" : "black");
        span.textContent = GLYPH[piece];
        span.draggable = !busy && piece === piece.toUpperCase();
        span.dataset.square = square;
        sq.append(span);
      }
      if (targets.has(square)) {
        const mark = document.createElement("span");
        mark.className = piece ? "ring" : "dot";
        sq.append(mark);
      }
      if (file === 0) sq.append(coord("rank", rank, "coord-rank"));
      if (rank === 1) sq.append(coord("file", FILES[file], "coord-file"));
      board.append(sq);
    }
  }
  renderStatus();
  renderMoves();
}

function coord(kind, value, cls) {
  const span = document.createElement("span");
  span.className = "coord " + cls;
  span.textContent = value;
  return span;
}

function findKing(pieces, whiteToMove) {
  const king = whiteToMove ? "K" : "k";
  return Object.keys(pieces).find((sq) => pieces[sq] === king) || null;
}

function renderStatus() {
  let text = { your_move: "ваш ход", engine_move: "движок думает", stalemate: "пат — ничья" }[state.status];
  if (!text && state.status === "mate_white") text = "мат — выиграли белые (вы)";
  if (!text && state.status === "mate_black") text = "мат — выиграли чёрные (движок)";
  if (!text && state.status.startsWith("draw:")) text = "ничья — " + state.status.slice(5);
  if (!text) text = state.status;
  if (busy) text = "движок думает…";
  statusEl.textContent = text + (state.check && !busy ? " · шах" : "");
}

function renderMoves() {
  movesEl.textContent = "";
  for (let i = 0; i < state.moves.length; i += 2) {
    const li = document.createElement("li");
    const white = state.moves[i] || "";
    const black = state.moves[i + 1] || "";
    li.textContent = `${white} ${black}`.trim();
    movesEl.append(li);
  }
  const info = state.info;
  if (!info) {
    infoEl.textContent = "—";
    return;
  }
  if (info.depth === 0) {
    infoEl.textContent = "случайный ход";
    return;
  }
  const score = info.mate
    ? "мат в " + Math.ceil((32000 - Math.abs(info.score)) / 2)
    : (info.score > 0 ? "+" : "") + (info.score / 100).toFixed(2);
  infoEl.textContent = `${info.move} · ${score} · глубина ${info.depth} · ${(info.nodes / 1000).toFixed(0)}k узлов · ${info.ms} мс`;
}

async function request(path, body) {
  const options = body === undefined
    ? { method: "GET" }
    : { method: "POST", headers: { "Content-Type": "application/x-www-form-urlencoded" }, body };
  const res = await fetch(path, options);
  return res.json();
}

function apply(data) {
  if (data.error) {
    statusEl.textContent = "ошибка: " + data.error;
    return;
  }
  state = data;
  selected = null;
  pending = null;
  hint = null;
  hintEl.hidden = true;
  promoEl.hidden = true;
  if (levelEl.options.length !== data.levels.length) {
    levelEl.textContent = "";
    for (const level of data.levels) {
      const option = document.createElement("option");
      option.value = level.id;
      option.textContent = level.label;
      levelEl.append(option);
    }
  }
  levelEl.value = data.level;
  board.dataset.turn = data.fen.split(" ")[1];
  render();
}

async function sendMove(from, to, promo) {
  busy = true;
  renderStatus();
  let body = `from=${from}&to=${to}`;
  if (promo) body += `&promo=${promo}`;
  apply(await request("/api/move", body));
  busy = false;
  render();
}

function onSquare(square) {
  if (busy || state.status !== "your_move") return;
  if (!selected) {
    if (legalFrom(square).length) selected = square;
    render();
    return;
  }
  if (square === selected) {
    selected = null;
    render();
    return;
  }
  const moves = state.legal.filter((m) => m.slice(0, 2) === selected && m.slice(2, 4) === square);
  if (moves.length === 0) {
    selected = legalFrom(square).length ? square : null;
    render();
    return;
  }
  if (moves.length > 1) {
    pending = { from: selected, to: square };
    promoEl.hidden = false;
    return;
  }
  sendMove(selected, square, moves[0].slice(4));
}

board.addEventListener("click", (event) => {
  const square = event.target.closest(".sq")?.dataset.square;
  if (square) onSquare(square);
});

board.addEventListener("dragstart", (event) => {
  const square = event.target.dataset.square;
  if (!square) return;
  event.dataTransfer.setData("text/plain", square);
  if (!selected) {
    selected = square;
    render();
  }
});
board.addEventListener("dragover", (event) => event.preventDefault());
board.addEventListener("drop", (event) => {
  event.preventDefault();
  const from = event.dataTransfer.getData("text/plain");
  const square = event.target.closest(".sq")?.dataset.square;
  if (!square) return;
  selected = from;
  onSquare(square);
});

promoEl.addEventListener("click", (event) => {
  const piece = event.target.dataset.piece;
  if (!piece || !pending) return;
  const { from, to } = pending;
  promoEl.hidden = true;
  sendMove(from, to, piece);
});

hintBtn.addEventListener("click", async () => {
  if (busy || state.status !== "your_move") return;
  busy = true;
  renderStatus();
  const data = await request("/api/hint", "");
  busy = false;
  if (data && data.move) {
    hint = { from: data.move.slice(0, 2), to: data.move.slice(2, 4) };
    const score = data.mate ? "мат" : (data.score > 0 ? "+" : "") + (data.score / 100).toFixed(2);
    hintEl.textContent = `подсказка: ${data.move} (${score}, глубина ${data.depth})`;
    hintEl.hidden = false;
  } else {
    hint = null;
    hintEl.hidden = true;
  }
  render();
});

newBtn.addEventListener("click", async () => {
  busy = true;
  renderStatus();
  apply(await request("/api/new", "level=" + levelEl.value));
  busy = false;
  render();
});

undoBtn.addEventListener("click", async () => {
  if (busy) return;
  apply(await request("/api/undo", ""));
});

levelEl.addEventListener("change", async () => {
  apply(await request("/api/level", "level=" + levelEl.value));
});

apply(await request("/api/state"));

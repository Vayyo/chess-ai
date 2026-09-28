//! Trains the engine's value network on datagen output with bullet.
//!
//! Architecture `(768 -> HIDDEN)x2 -> 1`, SCReLU, perspective accumulators.
//! Saved layout (little-endian, i16): l0w [768][HIDDEN] (QA), l0b [HIDDEN] (QA),
//! l1w [2 * HIDDEN] (QB; side to move first), l1b (QA * QB).
//!
//! Usage: trainer <net-id> <data.bin>... [--superbatches N] [--wdl F] [--lr F]

use bullet_lib::{
    game::inputs::Chess768,
    nn::optimiser::AdamW,
    trainer::{
        save::SavedFormat,
        schedule::{TrainingSchedule, TrainingSteps, lr, wdl},
        settings::LocalSettings,
    },
    value::{ValueTrainerBuilder, loader::DirectSequentialDataLoader},
};

/// Must match the engine's `nnue::HIDDEN`.
const HIDDEN: usize = 256;
/// Centipawns per sigmoid unit; must match the engine's `nnue::SCALE`.
const SCALE: i32 = 400;
const QA: i16 = 255;
const QB: i16 = 64;
const BATCH_SIZE: usize = 16_384;
const USAGE: &str = "usage: trainer <net-id> <data.bin>... [--epochs N] [--wdl F] [--lr F]";

struct Args {
    net_id: String,
    data: Vec<String>,
    /// One epoch is one pass over the training data.
    epochs: usize,
    /// Weight of the game result against the search score in the target.
    wdl: f32,
    lr: f32,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let net_id = args.next().ok_or("missing net id")?;
    let mut parsed = Args { net_id, data: Vec::new(), epochs: 20, wdl: 0.3, lr: 0.001 };
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--epochs" => parsed.epochs = value(&arg)?.parse().map_err(|e| format!("{arg}: {e}"))?,
            "--wdl" => parsed.wdl = value(&arg)?.parse().map_err(|e| format!("{arg}: {e}"))?,
            "--lr" => parsed.lr = value(&arg)?.parse().map_err(|e| format!("{arg}: {e}"))?,
            _ if arg.starts_with("--") => return Err(format!("unknown flag {arg}")),
            _ => parsed.data.push(arg),
        }
    }
    if parsed.data.is_empty() {
        return Err("no data files".into());
    }
    Ok(parsed)
}

fn main() {
    let args = parse_args().unwrap_or_else(|e| {
        eprintln!("{e}\n{USAGE}");
        std::process::exit(2);
    });

    let mut trainer = ValueTrainerBuilder::default()
        .dual_perspective()
        .optimiser(AdamW)
        .inputs(Chess768)
        .save_format(&[
            SavedFormat::id("l0w").round().quantise::<i16>(QA),
            SavedFormat::id("l0b").round().quantise::<i16>(QA),
            SavedFormat::id("l1w").round().quantise::<i16>(QB),
            SavedFormat::id("l1b").round().quantise::<i16>(QA * QB),
        ])
        // target = wdl * result + (1 - wdl) * sigmoid(score / SCALE)
        .loss_fn(|output, target| output.sigmoid().squared_error(target))
        .build(|builder, stm_inputs, ntm_inputs| {
            let l0 = builder.new_affine("l0", 768, HIDDEN);
            let l1 = builder.new_affine("l1", 2 * HIDDEN, 1);
            let stm = l0.forward(stm_inputs).screlu();
            let ntm = l0.forward(ntm_inputs).screlu();
            l1.forward(stm.concat(ntm))
        });

    // One schedule step is one pass over the data: 32 bytes per position.
    let positions: u64 = args.data.iter().map(|p| std::fs::metadata(p).expect("missing data file").len() / 32).sum();
    let batches_per_epoch = (positions / BATCH_SIZE as u64).max(1) as usize;
    println!("{positions} positions, {batches_per_epoch} batches/epoch, {} epochs", args.epochs);

    let schedule = TrainingSchedule {
        net_id: args.net_id,
        eval_scale: SCALE as f32,
        steps: TrainingSteps {
            batch_size: BATCH_SIZE,
            batches_per_superbatch: batches_per_epoch,
            start_superbatch: 1,
            end_superbatch: args.epochs,
        },
        wdl_scheduler: wdl::ConstantWDL { value: args.wdl },
        lr_scheduler: lr::CosineDecayLR {
            initial_lr: args.lr,
            final_lr: args.lr * 0.3f32.powi(5),
            final_superbatch: args.epochs,
        },
        save_rate: 5,
    };
    let settings =
        LocalSettings { threads: 4, test_set: None, output_directory: "checkpoints", batch_queue_size: 64 };
    let data: Vec<&str> = args.data.iter().map(String::as_str).collect();
    trainer.run(&schedule, &settings, &DirectSequentialDataLoader::new(&data));
}

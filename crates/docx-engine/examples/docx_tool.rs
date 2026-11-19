//! Developer tool for trying the engine on real files.
//!
//! ```text
//! cargo run -p docx-engine --example docx_tool -- inspect <file.docx>
//! cargo run -p docx-engine --example docx_tool -- model <file.docx>
//! cargo run -p docx-engine --example docx_tool -- edit <in.docx> <out.docx> [OPERATION]...
//! cargo run -p docx-engine --example docx_tool -- new <out.docx> [OPERATION]...
//! ```
//!
//! Operations, applied in order:
//! `--text <block> <text>`, `--insert <block> <text>`, `--delete <block>`,
//! `--style <block> <styleId>`, `--align <block> left|center|right|justify`.
//! With no operations, `edit` re-saves the file unchanged.

use std::process::ExitCode;

use docx_engine::Document;
use docx_engine::model::{Alignment, Block};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args {
        [cmd, path] if cmd == "inspect" => {
            let doc = open(path)?;
            let model = doc.model();
            println!("main part: {}", doc.main_part());
            println!("parts: {}", doc.package().parts().len());
            println!("stats: {:?}", model.stats);
            println!("section: {:?}", model.section);
            println!("headers: {:?}  footers: {:?}", model.headers, model.footers);
            println!("warnings: {:?}", model.warnings);
            for (i, block) in model.blocks.iter().enumerate() {
                let summary = match block {
                    Block::Paragraph(p) => format!(
                        "paragraph style={:?} align={:?} {:?}",
                        p.style_id,
                        p.alignment,
                        truncate(&p.plain_text(), 90)
                    ),
                    Block::Table(t) => format!("table rows={} style={:?}", t.rows.len(), t.style_id),
                    Block::Other { kind, text } => format!("other <{kind}> {}", truncate(text, 80)),
                };
                println!("[{i:>3}] {summary}");
            }
            Ok(())
        }
        [cmd, path] if cmd == "model" => {
            let model = open(path)?.model();
            println!("{}", serde_json::to_string_pretty(&model).map_err(|e| e.to_string())?);
            Ok(())
        }
        [cmd, input, output, ops @ ..] if cmd == "edit" => {
            let mut doc = open(input)?;
            apply(&mut doc, ops)?;
            let bytes = doc.save().map_err(|e| e.to_string())?;
            std::fs::write(output, bytes).map_err(|e| format!("{output}: {e}"))?;
            println!("wrote {output}");
            Ok(())
        }
        [cmd, output, ops @ ..] if cmd == "new" => {
            let mut doc = Document::new_blank();
            apply(&mut doc, ops)?;
            let bytes = doc.save().map_err(|e| e.to_string())?;
            std::fs::write(output, bytes).map_err(|e| format!("{output}: {e}"))?;
            println!("wrote {output}");
            Ok(())
        }
        _ => {
            Err("usage: docx_tool inspect|model <file> | edit <in> <out> [operations] | new <out> [operations]".into())
        }
    }
}

fn open(path: &str) -> Result<Document, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    Document::open(&bytes).map_err(|e| e.to_string())
}

fn apply(doc: &mut Document, mut ops: &[String]) -> Result<(), String> {
    let index = |s: &str| s.parse::<usize>().map_err(|_| format!("'{s}' is not a block number"));
    while !ops.is_empty() {
        let consumed = match ops {
            [op, block, text, ..] if op == "--text" => {
                doc.set_paragraph_text(index(block)?, text).map_err(|e| e.to_string())?;
                3
            }
            [op, block, text, ..] if op == "--insert" => {
                doc.insert_paragraph(index(block)?, text, None).map_err(|e| e.to_string())?;
                3
            }
            [op, block, ..] if op == "--delete" => {
                doc.delete_block(index(block)?).map_err(|e| e.to_string())?;
                2
            }
            [op, block, style, ..] if op == "--style" => {
                doc.set_paragraph_style(index(block)?, Some(style)).map_err(|e| e.to_string())?;
                3
            }
            [op, block, align, ..] if op == "--align" => {
                let alignment = match align.as_str() {
                    "left" => Alignment::Left,
                    "center" => Alignment::Center,
                    "right" => Alignment::Right,
                    "justify" => Alignment::Justify,
                    other => return Err(format!("unknown alignment '{other}'")),
                };
                doc.set_paragraph_alignment(index(block)?, Some(alignment)).map_err(|e| e.to_string())?;
                3
            }
            [op, ..] => return Err(format!("unknown or incomplete operation '{op}'")),
            [] => unreachable!(),
        };
        ops = &ops[consumed..];
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max).collect::<String>()) }
}

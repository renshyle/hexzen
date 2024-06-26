use std::{
    char::{self, REPLACEMENT_CHARACTER},
    cmp, fs,
    io::{self, stdin, Read},
};

use clap::Parser;
use crossterm::style::{Color, Colors};
use screen::Screen;

mod screen;
mod search;

pub const BYTES_PER_ROW: usize = 16;

enum CursorMovementType {
    Right,
    Left,
    Up,
    Down,
    PageUp,
    PageDown,
}

#[derive(PartialEq)]
pub enum EditorMode {
    HexMode,
    TextMode,
}

impl EditorMode {
    pub fn next(&self) -> EditorMode {
        match self {
            EditorMode::HexMode => EditorMode::TextMode,
            EditorMode::TextMode => EditorMode::HexMode,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            EditorMode::HexMode => "normal",
            EditorMode::TextMode => "text",
        }
    }
}

struct FileEditor {
    buffer: Vec<u8>,
    filename: String,
    offset: usize,
    cursor_nibble: usize,
    saved: bool,
    undo_stack: Vec<Edit>,
    redo_stack: Vec<Edit>,
}

struct Edit {
    position: usize,
    prev_byte: u8,
    new_byte: u8,
}

impl FileEditor {
    pub fn new(filename: &str) -> Result<FileEditor, io::Error> {
        let buffer = fs::read(filename)?;

        Ok(FileEditor {
            buffer,
            filename: filename.to_owned(),
            offset: 0,
            cursor_nibble: 0,
            saved: true,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        })
    }

    pub fn file_size(&self) -> usize {
        self.buffer.len()
    }

    pub fn read_bytes(&self, size: usize) -> &[u8] {
        &self.buffer[self.offset..=cmp::min(self.offset + size, self.buffer.len() - 1)]
    }

    pub fn write_nibble(&mut self, nibble: u8) -> Result<(), io::Error> {
        let position = self.cursor_nibble / 2;
        let byte = self.buffer[position];

        let new_byte = if self.cursor_nibble % 2 == 0 {
            (byte & 0x0f) | (nibble << 4)
        } else {
            (byte & 0xf0) | (nibble & 0x0f)
        };

        self.push_undo(Edit {
            position,
            prev_byte: byte,
            new_byte,
        });

        self.buffer[self.cursor_nibble / 2] = new_byte;
        self.saved = false;

        Ok(())
    }

    pub fn write_byte(&mut self, byte: u8) -> Result<(), io::Error> {
        let position = self.cursor_nibble / 2;

        self.push_undo(Edit {
            position,
            prev_byte: self.buffer[position],
            new_byte: byte,
        });

        self.buffer[position] = byte;
        self.saved = false;

        Ok(())
    }

    pub fn push_undo(&mut self, edit: Edit) {
        self.undo_stack.push(edit);
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) -> bool {
        if let Some(edit) = self.undo_stack.pop() {
            self.buffer[edit.position] = edit.prev_byte;
            self.cursor_nibble = 2 * edit.position;
            self.redo_stack.push(edit);
            self.saved = false;

            true
        } else {
            false
        }
    }

    pub fn redo(&mut self) -> bool {
        if let Some(edit) = self.redo_stack.pop() {
            self.buffer[edit.position] = edit.new_byte;
            self.cursor_nibble = 2 * edit.position;
            self.undo_stack.push(edit);
            self.saved = false;

            true
        } else {
            false
        }
    }

    pub fn save(&mut self) -> Result<(), io::Error> {
        fs::write(&self.filename, &self.buffer)?;
        self.saved = true;

        Ok(())
    }
}

fn hexdump(file: &str, config: Config) -> Result<(), io::Error> {
    let buffer = if file == "-" {
        let mut buf = Vec::new();
        stdin().lock().read_to_end(&mut buf)?;
        buf
    } else {
        fs::read(file)?
    };
    let rows = (buffer.len() + BYTES_PER_ROW - 1) / BYTES_PER_ROW;

    println!("            00 01 02 03 04 05 06 07  08 09 0a 0b 0c 0d 0e 0f\n");

    for row in 0..rows {
        print!(" {:08x}   ", row * BYTES_PER_ROW);

        for col in 0..BYTES_PER_ROW {
            if col == 8 {
                print!(" ");
            }

            if row * BYTES_PER_ROW + col >= buffer.len() {
                print!("   ");
            } else {
                let c = buffer[row * BYTES_PER_ROW + col];

                print!("{:02x} ", c);
            }
        }

        print!("  ");

        for col in 0..BYTES_PER_ROW {
            if row * BYTES_PER_ROW + col < buffer.len() {
                let mut c = buffer[row * BYTES_PER_ROW + col] as char;

                if !(32..=126).contains(&(c as u8)) {
                    c = config.replacement_char;
                }

                print!("{}", c);
            }
        }

        println!();
    }

    Ok(())
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    file: String,
    #[arg(short, long, help = "prints a hex dump instead of opening the editor")]
    dump: bool,
    #[arg(
        short,
        help = "use the unicode replacement character instead of a dot when a character isn't printable ascii"
    )]
    unicode_replacement_char: bool,
    #[arg(short = 'c', long, help = "disables the use of colors in the editor")]
    no_colors: bool,
}

pub struct Config {
    replacement_char: char,
    highlight_colors: Option<Colors>,
}

fn main() {
    let args = Args::parse();
    let config = Config {
        replacement_char: match args.unicode_replacement_char {
            true => REPLACEMENT_CHARACTER,
            false => '.',
        },
        highlight_colors: match args.no_colors {
            true => None,
            false => Some(Colors::new(Color::White, Color::DarkGrey)),
        },
    };

    if !args.dump {
        let mut screen = Screen::new(&args.file, config).unwrap();

        screen.screen_loop().unwrap();
    } else {
        hexdump(&args.file, config).unwrap();
    }
}

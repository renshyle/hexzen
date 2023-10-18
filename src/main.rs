use core::fmt;
use std::{
    char::{self, REPLACEMENT_CHARACTER},
    cmp, fs,
    io::Write,
    io::{self, Stdout},
    io::{stdout, Error, ErrorKind},
};

use bytesize::ByteSize;
use clap::Parser;
use crossterm::{
    cursor,
    event::{read, Event, KeyCode},
    execute, queue, terminal,
    tty::IsTty,
};

const SCREEN_WIDTH: usize = 80;
const BYTES_PER_ROW: isize = 16;

struct Screen {
    editor: FileEditor,
    stdout: Stdout,
    width: isize,
    height: isize,
    editor_mode: EditorMode,
    config: Config,
}

enum CursorMovementType {
    Right,
    Left,
    Up,
    Down,
    PageUp,
    PageDown,
}

#[derive(PartialEq)]
enum EditorMode {
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
}

impl fmt::Display for EditorMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                EditorMode::HexMode => "normal",
                EditorMode::TextMode => "text",
            }
        )
    }
}

struct FileEditor {
    buffer: Vec<u8>,
    filename: String,
    offset: isize,
    cursor_nibble: isize,
    saved: bool,
    undo_stack: Vec<Edit>,
    redo_stack: Vec<Edit>,
}

struct Edit {
    position: usize,
    prev_byte: u8,
    new_byte: u8,
}

impl Screen {
    pub fn new(filename: &str, config: Config) -> Result<Screen, io::Error> {
        let stdout = stdout();

        if !stdout.is_tty() {
            return Err(Error::new(ErrorKind::Other, "not a terminal"));
        }

        let editor = FileEditor::new(filename)?;
        let (width, height) = terminal::size()?;

        Ok(Screen {
            editor,
            stdout,
            width: width.try_into().unwrap(),
            height: height.try_into().unwrap(),
            editor_mode: EditorMode::HexMode,
            config,
        })
    }

    pub fn screen_loop(&mut self) -> Result<(), io::Error> {
        terminal::enable_raw_mode()?;
        queue!(self.stdout, terminal::EnterAlternateScreen)?;
        queue!(self.stdout, terminal::Clear(terminal::ClearType::All))?;
        self.draw()?;

        loop {
            match read()? {
                Event::Key(event) => match event.code {
                    KeyCode::Char(c) => {
                        if let ' '..='~' = c {
                            match self.editor_mode {
                                EditorMode::HexMode => match c {
                                    'a'..='f' | '0'..='9' => {
                                        let nibble =
                                            u8::from_str_radix(&String::from(c), 16).unwrap();
                                        self.editor.write_nibble(nibble)?;
                                        self.move_cursor(CursorMovementType::Right)?;
                                    }
                                    'u' | 'z' => {
                                        let undid = self.editor.undo();

                                        if undid {
                                            self.draw()?;
                                        }
                                    }
                                    'r' => {
                                        let redid = self.editor.redo();

                                        if redid {
                                            self.draw()?;
                                        }
                                    }
                                    'w' => {
                                        if let Err(e) = self.editor.save() {
                                            eprintln!("unable to save file: {:?}", e);
                                        }

                                        self.draw()?;
                                    }
                                    'q' => {
                                        break;
                                    }
                                    _ => {}
                                },
                                EditorMode::TextMode => {
                                    if let ' '..='~' = c {
                                        self.editor.write_byte(c.to_string().as_bytes()[0])?;
                                        self.move_cursor(CursorMovementType::Right)?;
                                    }
                                }
                            }
                        }
                    }
                    KeyCode::Right => {
                        self.move_cursor(CursorMovementType::Right)?;
                    }
                    KeyCode::Left | KeyCode::Backspace => {
                        self.move_cursor(CursorMovementType::Left)?;
                    }
                    KeyCode::Down => {
                        self.move_cursor(CursorMovementType::Down)?;
                    }
                    KeyCode::Up => {
                        self.move_cursor(CursorMovementType::Up)?;
                    }
                    KeyCode::PageDown => {
                        self.move_cursor(CursorMovementType::PageDown)?;
                    }
                    KeyCode::PageUp => {
                        self.move_cursor(CursorMovementType::PageUp)?;
                    }
                    KeyCode::Tab => {
                        self.cycle_editor_mode()?;
                    }
                    KeyCode::Esc => {
                        self.set_editor_mode(EditorMode::HexMode)?;
                    }
                    _ => {}
                },
                Event::Resize(new_width, new_height) => {
                    self.width = new_width.try_into().unwrap();
                    self.height = new_height.try_into().unwrap();

                    queue!(self.stdout, terminal::Clear(terminal::ClearType::All))?;
                    self.draw()?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn draw(&mut self) -> Result<(), io::Error> {
        if self.height < 5 {
            return Ok(());
        }

        queue!(self.stdout, cursor::MoveTo(12, 1))?;
        write!(
            self.stdout,
            "00 01 02 03 04 05 06 07  08 09 0a 0b 0c 0d 0e 0f"
        )?;

        self.editor.cursor_nibble = self
            .editor
            .cursor_nibble
            .clamp(0, 2 * self.editor.file_size() - 1);

        let cursor_row = (self.editor.cursor_nibble / (2 * BYTES_PER_ROW)) * BYTES_PER_ROW;
        if self.editor.cursor_nibble < 2 * self.editor.offset {
            self.editor.offset = cursor_row;
        } else if self.editor.cursor_nibble
            >= 2 * (self.editor.offset + BYTES_PER_ROW * (self.height - 4))
        {
            self.editor.offset = cursor_row - BYTES_PER_ROW * (self.height - 4 - 1);
        }

        self.editor.offset = self.editor.offset.clamp(
            0,
            self.editor.file_size() - self.editor.file_size() % BYTES_PER_ROW,
        );

        let buf = self
            .editor
            .read_bytes((BYTES_PER_ROW * (self.height - 4)) as usize);

        let data_rows = cmp::min(
            self.height - 4,
            (self.editor.file_size() - self.editor.offset + BYTES_PER_ROW - 1) / BYTES_PER_ROW,
        );

        for row in 0..self.height - 4 {
            let y = (row + 3).try_into().unwrap();

            queue!(self.stdout, cursor::MoveTo(1, y))?;
            if row < data_rows {
                write!(
                    self.stdout,
                    "{:08x}",
                    (self.editor.offset + row * 0x10) & 0xfffffff0
                )?;
            } else {
                write!(self.stdout, "        ")?;
            }

            queue!(self.stdout, cursor::MoveTo(63, y))?;
            for col in 0..BYTES_PER_ROW {
                if self.editor.offset + row * BYTES_PER_ROW + col >= self.editor.file_size() {
                    write!(self.stdout, " ")?;
                } else {
                    let mut c = buf[(row * BYTES_PER_ROW + col) as usize] as char;

                    if !(32..=126).contains(&(c as u8)) {
                        c = self.config.replacement_char;
                    }

                    write!(self.stdout, "{}", c)?;
                }
            }

            queue!(self.stdout, cursor::MoveTo(12, y))?;
            for col in 0..BYTES_PER_ROW {
                if self.editor.offset + row * BYTES_PER_ROW + col >= self.editor.file_size() {
                    write!(self.stdout, "   ")?;
                } else {
                    let c = buf[(row * BYTES_PER_ROW + col) as usize];

                    if col == 8 {
                        write!(self.stdout, " ")?;
                    }

                    write!(self.stdout, "{:02x} ", c)?;
                };
            }
        }

        queue!(self.stdout, cursor::MoveTo(1, 0))?;
        queue!(
            self.stdout,
            terminal::Clear(terminal::ClearType::CurrentLine)
        )?;

        let mode = self.editor_mode.to_string();
        queue!(
            self.stdout,
            cursor::MoveTo((SCREEN_WIDTH - mode.len()).try_into().unwrap(), 0)
        )?;
        write!(self.stdout, "{}", &mode)?;

        queue!(self.stdout, cursor::MoveTo(0, self.height as u16 - 1))?;
        queue!(
            self.stdout,
            terminal::Clear(terminal::ClearType::CurrentLine)
        )?;
        write!(
            self.stdout,
            "[{}] {}",
            self.editor.filename,
            ByteSize::b(self.editor.file_size() as u64)
        )?;

        if !self.editor.saved {
            write!(self.stdout, " [+]")?;
        }

        self.draw_cursor()?;

        self.stdout.flush()?;

        Ok(())
    }

    pub fn cycle_editor_mode(&mut self) -> Result<(), io::Error> {
        self.set_editor_mode(self.editor_mode.next())
    }

    pub fn set_editor_mode(&mut self, editor_mode: EditorMode) -> Result<(), io::Error> {
        if self.editor_mode != editor_mode {
            self.editor_mode = editor_mode;
            self.editor.cursor_nibble -= self.editor.cursor_nibble % 2;
            self.draw()
        } else {
            Ok(())
        }
    }

    fn move_cursor(&mut self, movement: CursorMovementType) -> Result<(), io::Error> {
        let xmov = match self.editor_mode {
            EditorMode::HexMode => 1,
            EditorMode::TextMode => 2,
        };

        let ymov = 2 * BYTES_PER_ROW;

        match movement {
            CursorMovementType::Right => {
                self.editor.cursor_nibble += xmov;
            }
            CursorMovementType::Left => {
                self.editor.cursor_nibble -= xmov;
            }
            CursorMovementType::Up => {
                self.editor.cursor_nibble -= ymov;
            }
            CursorMovementType::Down => {
                self.editor.cursor_nibble += ymov;
            }
            CursorMovementType::PageUp => {
                self.editor.cursor_nibble -= ymov * (self.height - 4);
                self.editor.offset -= (self.height - 4) * BYTES_PER_ROW;
            }
            CursorMovementType::PageDown => {
                self.editor.cursor_nibble += ymov * (self.height - 4);
                self.editor.offset += (self.height - 4) * BYTES_PER_ROW;
            }
        }

        self.draw()?;

        Ok(())
    }

    fn draw_cursor(&mut self) -> Result<(), io::Error> {
        let (x, y) = self.coords_for_cursor();
        queue!(
            self.stdout,
            cursor::MoveTo(x.try_into().unwrap(), y.try_into().unwrap())
        )
    }

    fn coords_for_cursor(&self) -> (isize, isize) {
        let nibble_wo_offset = self.editor.cursor_nibble - 2 * self.editor.offset;

        match self.editor_mode {
            EditorMode::HexMode => {
                let mut x =
                    12 + (nibble_wo_offset % (2 * BYTES_PER_ROW)) / 2 * 3 + nibble_wo_offset % 2;
                let y = 3 + nibble_wo_offset / (2 * BYTES_PER_ROW);

                if x >= 36 {
                    x += 1;
                }

                (x, y)
            }
            EditorMode::TextMode => {
                let x = 63 + (nibble_wo_offset % (2 * BYTES_PER_ROW)) / 2;
                let y = 3 + nibble_wo_offset / (2 * BYTES_PER_ROW);

                (x, y)
            }
        }
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        terminal::disable_raw_mode().unwrap();
        execute!(self.stdout, terminal::LeaveAlternateScreen).unwrap();
    }
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

    pub fn file_size(&self) -> isize {
        self.buffer.len() as isize
    }

    pub fn read_bytes(&self, size: usize) -> &[u8] {
        &self.buffer
            [self.offset as usize..=cmp::min(self.offset as usize + size, self.buffer.len() - 1)]
    }

    pub fn write_nibble(&mut self, nibble: u8) -> Result<(), io::Error> {
        let position = (self.cursor_nibble / 2) as usize;
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

        self.buffer[(self.cursor_nibble / 2) as usize] = new_byte;
        self.saved = false;

        Ok(())
    }

    pub fn write_byte(&mut self, byte: u8) -> Result<(), io::Error> {
        let position = (self.cursor_nibble / 2) as usize;

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
            self.cursor_nibble = 2 * edit.position as isize;
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
            self.cursor_nibble = 2 * edit.position as isize;
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
    let buffer = fs::read(file)?;
    let rows = (buffer.len() as isize + BYTES_PER_ROW - 1) / BYTES_PER_ROW;

    println!("            00 01 02 03 04 05 06 07  08 09 0a 0b 0c 0d 0e 0f\n");

    for row in 0..rows {
        print!(" {:08x}   ", row * BYTES_PER_ROW);

        for col in 0..BYTES_PER_ROW {
            if col == 8 {
                print!(" ");
            }

            if row * BYTES_PER_ROW + col >= buffer.len() as isize {
                print!("   ");
            } else {
                let c = buffer[(row * BYTES_PER_ROW + col) as usize];

                print!("{:02x} ", c);
            }
        }

        print!("  ");

        for col in 0..BYTES_PER_ROW {
            if row * BYTES_PER_ROW + col < buffer.len() as isize {
                let mut c = buffer[(row * BYTES_PER_ROW + col) as usize] as char;

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
}

struct Config {
    replacement_char: char,
}

fn main() {
    let args = Args::parse();
    let config = Config {
        replacement_char: match args.unicode_replacement_char {
            true => REPLACEMENT_CHARACTER,
            false => '.',
        },
    };

    if !args.dump {
        let mut screen = Screen::new(&args.file, config).unwrap();

        screen.screen_loop().unwrap();
    } else {
        hexdump(&args.file, config).unwrap();
    }
}

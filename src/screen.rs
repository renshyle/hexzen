use std::{
    io::{self, stdout, ErrorKind, Stdout, Write},
    mem,
};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute, queue, style, terminal,
    tty::IsTty,
};

use std::{char, cmp};

use bytesize::ByteSize;

use crate::{
    search::{self, SearchResults},
    Config, CursorMovementType, EditorMode, FileEditor, BYTES_PER_ROW,
};

const SCREEN_WIDTH: usize = 80;

type InputReadCallback = Box<dyn FnMut(&mut Screen, &str)>;

pub struct Screen {
    editor: FileEditor,
    running: bool,
    stdout: Stdout,
    width: usize,
    height: usize,
    editor_mode: EditorMode,
    screen_mode: ScreenMode,
    input_buffer: Vec<char>,
    input_callback: Option<InputReadCallback>,
    input_prefix: String,
    search_results: Option<SearchResults>,
    config: Config,
}

enum ScreenMode {
    EditMode,
    CommandMode,
}

impl Screen {
    pub fn new(filename: &str, config: Config) -> Result<Screen, io::Error> {
        let stdout = stdout();

        if !stdout.is_tty() {
            return Err(io::Error::new(ErrorKind::Other, "not a terminal"));
        }

        let editor = FileEditor::new(filename)?;
        let (width, height) = terminal::size()?;

        Ok(Screen {
            editor,
            running: true,
            stdout,
            width: width.into(),
            height: height.into(),
            editor_mode: EditorMode::HexMode,
            screen_mode: ScreenMode::EditMode,
            input_buffer: Vec::new(),
            input_callback: None,
            input_prefix: String::new(),
            search_results: None,
            config,
        })
    }

    pub fn screen_loop(&mut self) -> Result<(), io::Error> {
        terminal::enable_raw_mode()?;
        queue!(self.stdout, terminal::EnterAlternateScreen)?;
        queue!(self.stdout, terminal::Clear(terminal::ClearType::All))?;
        self.draw()?;

        while self.running {
            match event::read()? {
                Event::Key(event) => match self.screen_mode {
                    ScreenMode::EditMode => match event.code {
                        KeyCode::Char(c) => match self.editor_mode {
                            EditorMode::HexMode => match c {
                                'a'..='f' | '0'..='9' => {
                                    let nibble = hex_char_to_u8(c).unwrap();
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
                                'j' => {
                                    self.read_user_input(
                                        String::from("j "),
                                        Box::new(|screen: &mut Screen, mut input: &str| {
                                            input = input.strip_prefix("0x").unwrap_or(input);

                                            if let Ok(address) = usize::from_str_radix(input, 16) {
                                                screen.editor.cursor_nibble = 2 * address;
                                            }
                                        }),
                                    )?;
                                }
                                '/' => {
                                    self.search_results = None;

                                    self.read_user_input(
                                        String::from("/"),
                                        Box::new(|screen: &mut Screen, input: &str| {
                                            screen.search_results =
                                                search::search(&screen.editor.buffer, input);

                                            if let Some(results) = &screen.search_results {
                                                screen.editor.cursor_nibble = 2 * results.result();
                                            }
                                        }),
                                    )?;
                                }
                                'n' => {
                                    if let Some(search_results) = &mut self.search_results {
                                        self.editor.cursor_nibble = 2 * search_results.next();
                                        self.draw()?;
                                    }
                                }
                                'm' => {
                                    if let Some(search_results) = &mut self.search_results {
                                        self.editor.cursor_nibble = 2 * search_results.prev();
                                        self.draw()?;
                                    }
                                }
                                'q' => {
                                    if self.editor.saved {
                                        self.running = false;
                                    } else {
                                        self.read_user_input(
                                            String::from("quit without saving? "),
                                            Box::new(|screen: &mut Screen, input: &str| {
                                                if input.eq_ignore_ascii_case("yes")
                                                    || input.eq_ignore_ascii_case("y")
                                                {
                                                    screen.running = false;
                                                }
                                            }),
                                        )?;
                                    }
                                }
                                _ => {}
                            },
                            EditorMode::TextMode => {
                                if let ' '..='~' = c {
                                    self.editor.write_byte(c as u8)?;
                                    self.move_cursor(CursorMovementType::Right)?;
                                }
                            }
                        },
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
                    ScreenMode::CommandMode => match event.code {
                        KeyCode::Char(c) => {
                            self.input_buffer.push(c);
                            self.draw()?;
                        }
                        KeyCode::Esc => {
                            self.screen_mode = ScreenMode::EditMode;
                            self.input_buffer.clear();
                            self.input_callback = None;
                            self.draw()?;
                        }
                        KeyCode::Backspace => {
                            if self.input_buffer.is_empty() {
                                self.screen_mode = ScreenMode::EditMode;
                            } else {
                                self.input_buffer.remove(self.input_buffer.len() - 1);
                            }

                            self.draw()?;
                        }
                        KeyCode::Enter => {
                            let command: String =
                                mem::take(&mut self.input_buffer).into_iter().collect();

                            let callback = self.input_callback.take();
                            if let Some(mut callback) = callback {
                                callback(self, &command);
                            }

                            self.screen_mode = ScreenMode::EditMode;
                            self.draw()?;
                        }
                        _ => {}
                    },
                },
                Event::Resize(new_width, new_height) => {
                    self.width = new_width.into();
                    self.height = new_height.into();

                    queue!(self.stdout, terminal::Clear(terminal::ClearType::All))?;
                    self.draw()?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn read_user_input(
        &mut self,
        prefix: String,
        callback: InputReadCallback,
    ) -> Result<(), io::Error> {
        self.screen_mode = ScreenMode::CommandMode;
        self.input_callback = Some(callback);
        self.input_prefix = prefix;
        self.draw()
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

        let buf = self.editor.read_bytes(BYTES_PER_ROW * (self.height - 4));

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
                let offset = self.editor.offset + row * BYTES_PER_ROW + col;

                if offset >= self.editor.file_size() {
                    write!(self.stdout, " ")?;
                } else {
                    let mut c = buf[row * BYTES_PER_ROW + col] as char;

                    if !(32..=126).contains(&(c as u8)) {
                        c = self.config.replacement_char;
                    }

                    let match_len = self
                        .search_results
                        .as_ref()
                        .and_then(|res| res.match_len(offset));
                    let highlight = self.config.highlight_colors.is_some() && match_len.is_some();

                    if highlight {
                        queue!(
                            self.stdout,
                            style::SetColors(self.config.highlight_colors.unwrap())
                        )?;
                        write!(self.stdout, "{}", c)?;
                        queue!(self.stdout, style::ResetColor)?;
                    } else {
                        write!(self.stdout, "{}", c)?;
                    }
                }
            }

            queue!(self.stdout, cursor::MoveTo(12, y))?;
            for col in 0..BYTES_PER_ROW {
                let offset = self.editor.offset + row * BYTES_PER_ROW + col;

                if offset >= self.editor.file_size() {
                    write!(self.stdout, "   ")?;
                } else {
                    let c = buf[row * BYTES_PER_ROW + col];

                    let match_len = self
                        .search_results
                        .as_ref()
                        .and_then(|res| res.match_len(offset));
                    let highlight = self.config.highlight_colors.is_some() && match_len.is_some();

                    if highlight {
                        queue!(
                            self.stdout,
                            style::SetColors(self.config.highlight_colors.unwrap())
                        )?;
                        if col == BYTES_PER_ROW - 1 || match_len.unwrap() == 1 {
                            write!(self.stdout, "{:02x}", c)?;
                            queue!(self.stdout, style::ResetColor)?;

                            if col == 7 {
                                write!(self.stdout, "  ")?;
                            } else {
                                write!(self.stdout, " ")?;
                            }
                        } else {
                            write!(self.stdout, "{:02x} ", c)?;

                            if col == 7 {
                                write!(self.stdout, " ")?;
                            }

                            queue!(self.stdout, style::ResetColor)?;
                        }
                    } else {
                        write!(self.stdout, "{:02x} ", c)?;

                        if col == 7 {
                            write!(self.stdout, " ")?;
                        }
                    }
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

        match self.screen_mode {
            ScreenMode::EditMode => {
                write!(
                    self.stdout,
                    "[{}] {}",
                    self.editor.filename,
                    ByteSize::b(self.editor.file_size() as u64)
                )?;

                if !self.editor.saved {
                    write!(self.stdout, " [+]")?;
                }

                if let Some(search_results) = &self.search_results {
                    write!(
                        self.stdout,
                        " [{}/{}]",
                        search_results.idx() + 1,
                        search_results.len()
                    )?;
                }
            }
            ScreenMode::CommandMode => {
                write!(
                    self.stdout,
                    "{}{}",
                    self.input_prefix,
                    self.input_buffer[(self.input_buffer.len() + self.input_prefix.len() + 1)
                        .saturating_sub(self.width)
                        ..self.input_buffer.len()]
                        .iter()
                        .collect::<String>()
                )?;
            }
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
                self.editor.cursor_nibble = self
                    .editor
                    .cursor_nibble
                    .saturating_sub(ymov * (self.height - 4));
                self.editor.offset = self
                    .editor
                    .offset
                    .saturating_sub((self.height - 4) * BYTES_PER_ROW);
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
        let (x, y) = match self.screen_mode {
            ScreenMode::EditMode => self.coords_for_cursor(),
            ScreenMode::CommandMode => (
                (self.input_prefix.len() + self.input_buffer.len()),
                self.height - 1,
            ),
        };
        queue!(
            self.stdout,
            cursor::MoveTo(x.try_into().unwrap(), y.try_into().unwrap())
        )
    }

    fn coords_for_cursor(&self) -> (usize, usize) {
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

fn hex_char_to_u8(c: char) -> Option<u8> {
    let i = c as u8;

    match c {
        '0'..='9' => Some(i - b'0'),
        'a'..='f' => Some(i - b'a' + 10),
        _ => None,
    }
}

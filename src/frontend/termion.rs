use crossbeam_channel::{self, Receiver};
use std::{
    fmt::{self, Display, Formatter},
    io::{self, BufWriter, Read, Stdout, Write},
    thread::{self, JoinHandle},
};
use termion::{
    self,
    cursor::Goto,
    event::Key as TermionKey,
    input::TermRead,
    raw::{IntoRawMode, RawTerminal},
    screen::AlternateScreen,
};

use super::{Frontend, Result};
use crate::terminal::{screen::Textel, Colour, Key, Screen, Size, Style};

pub type Error = std::io::Error;

pub struct Termion {
    target: AlternateScreen<RawTerminal<BufWriter<Stdout>>>,
    input: Input,
}

impl Termion {
    pub fn new() -> Result<Self> {
        let mut target =
            AlternateScreen::from(BufWriter::with_capacity(1 << 20, io::stdout()).into_raw_mode()?);
        write!(target, "{}", termion::cursor::Hide)?;

        Ok(Self {
            target,
            input: Input::from_reader(termion::get_tty()?),
        })
    }
}

impl Frontend for Termion {
    #[inline]
    fn size(&self) -> Result<Size> {
        let (width, height) = termion::terminal_size()?;
        Ok(Size::new(width as usize, height as usize))
    }

    #[inline]
    fn present(&mut self, screen: &Screen) -> Result<()> {
        let Self { ref mut target, .. } = *self;

        let mut last_style = Style::default();
        write!(target, "{}", last_style)?;

        screen
            .buffer()
            .chunks(screen.size().width)
            .enumerate()
            .try_for_each(|(y, line)| {
                // Go to the begining of line (`Goto` uses 1-based indexing)
                write!(target, "{}", Goto(1, (y + 1) as u16))?;

                line.iter().try_for_each(|textel| -> Result<()> {
                    if let Some(Textel {
                        ref style,
                        ref content,
                    }) = textel
                    {
                        if *style != last_style {
                            write!(target, "{}", style)?;
                            last_style = *style;
                        }
                        write!(target, "{}", content)?;
                    }
                    Ok(())
                })
            })?;

        target.flush()?;
        Ok(())
    }

    #[inline]
    fn events(&self) -> &Receiver<Key> {
        &self.input.receiver
    }
}

impl Drop for Termion {
    fn drop(&mut self) {
        write!(
            self.target,
            "{}{}{}{}{}",
            termion::color::Fg(termion::color::Reset),
            termion::color::Bg(termion::color::Reset),
            termion::clear::All,
            termion::cursor::Show,
            termion::screen::ToMainScreen
        )
        .expect("clear screen on drop");
    }
}

impl Display for Style {
    #[inline]
    fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
        // Bold
        if self.bold {
            write!(formatter, "{}", termion::style::Bold)?;
        } else {
            // Using Reset is not ideal as it resets all style attributes. The correct thing to do
            // would be to use `NoBold`, but it seems this is not reliably supported (at least it
            // didn't work for me in tmux, although it does in alacritty).
            // Also see https://github.com/crossterm-rs/crossterm/issues/294
            write!(formatter, "{}", termion::style::Reset)?;
        }

        // Underline
        if self.underline {
            write!(formatter, "{}", termion::style::Underline)?;
        } else {
            write!(formatter, "{}", termion::style::NoUnderline)?;
        }

        // Background
        {
            let Colour { red, green, blue } = self.background.0;
            write!(
                formatter,
                "{}",
                termion::color::Bg(termion::color::Rgb(red, green, blue))
            )?;
        }

        // Foreground
        {
            let Colour { red, green, blue } = self.foreground.0;
            write!(
                formatter,
                "{}",
                termion::color::Fg(termion::color::Rgb(red, green, blue))
            )?;
        }

        Ok(())
    }
}

struct Input {
    receiver: Receiver<Key>,
    _handle: JoinHandle<()>,
}

impl Input {
    pub fn from_reader(reader: impl Read + Send + 'static) -> Self {
        let (sender, receiver) = crossbeam_channel::bounded(2048);
        let _handle = thread::spawn(move || {
            for event in reader.keys() {
                match event {
                    Ok(termion_key) => {
                        sender.send(map_key(termion_key)).unwrap();
                    }
                    error => {
                        error.unwrap();
                    }
                }
            }
        });
        Self { receiver, _handle }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        // ??
    }
}

#[inline]
fn map_key(key: TermionKey) -> Key {
    match key {
        TermionKey::Backspace => Key::Backspace,
        TermionKey::Left => Key::Left,
        TermionKey::Right => Key::Right,
        TermionKey::Up => Key::Up,
        TermionKey::Down => Key::Down,
        TermionKey::Home => Key::Home,
        TermionKey::End => Key::End,
        TermionKey::PageUp => Key::PageUp,
        TermionKey::PageDown => Key::PageDown,
        TermionKey::BackTab => Key::BackTab,
        TermionKey::Delete => Key::Delete,
        TermionKey::Insert => Key::Insert,
        TermionKey::F(u8) => Key::F(u8),
        TermionKey::Char(char) => Key::Char(char),
        TermionKey::Alt(char) => Key::Alt(char),
        TermionKey::Ctrl(char) => Key::Ctrl(char),
        TermionKey::Null => Key::Null,
        TermionKey::Esc => Key::Esc,
        _ => panic!("Unknown termion key event: {:?}", key),
    }
}

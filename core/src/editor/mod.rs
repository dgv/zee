pub mod buffer;
mod windows;

pub use self::buffer::{BufferId, ModifiedStatus};

use git2::Repository;
use ropey::Rope;
use std::{
    borrow::Cow,
    fmt::Display,
    fs::File,
    io::{self, BufReader},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};
use zi::{
    BindingMatch, BindingTransition, Component, ComponentExt, ComponentLink, FlexBasis,
    FlexDirection, Item, Key, Layout, Rect, ShouldRender,
};

use crate::{
    clipboard::Clipboard,
    components::{
        buffer::{Buffer as BufferView, Properties as BufferViewProperties},
        prompt::{
            buffers::BufferEntry, picker::FileSource, Action as PromptAction, Prompt,
            Properties as PromptProperties, PROMPT_INACTIVE_HEIGHT,
        },
        splash::{Properties as SplashProperties, Splash},
        theme::{Theme, THEMES},
    },
    error::Result,
    settings::Settings,
    task::TaskPool,
};

use self::{
    buffer::{BufferCursor, Buffers, BuffersMessage, CursorId, RepositoryRc},
    windows::{CycleFocus, Window, WindowTree},
};

#[derive(Debug)]
pub enum Message {
    ChangeTheme,
    ClosePane,
    FocusNextComponent,
    FocusPreviousComponent,
    SplitWindow(FlexDirection),
    FullscreenWindow,
    KeyPressed,
    OpenBufferSwitcher,
    ChangePromptHeight(usize),
    OpenFilePicker(FileSource),
    OpenFile(PathBuf),
    SelectBuffer(BufferId),
    Buffer(BuffersMessage),
    Log(Option<String>),
    Cancel,
    Quit,
}

impl From<BuffersMessage> for Message {
    fn from(message: BuffersMessage) -> Message {
        Message::Buffer(message)
    }
}

pub struct Context {
    pub args_files: Vec<PathBuf>,
    pub current_working_dir: PathBuf,
    pub settings: Settings,
    pub task_pool: TaskPool,
    pub clipboard: Arc<dyn Clipboard>,
}

#[derive(Clone)]
pub struct Logger {
    link: ComponentLink<Editor>,
}

impl Logger {
    fn new(link: ComponentLink<Editor>) -> Self {
        Self { link }
    }

    pub fn info(&self, message: String) {
        self.link.send(Message::Log(Some(message)));
    }
}

pub struct Editor {
    context: Rc<Context>,
    link: ComponentLink<Self>,
    themes: &'static [(Theme, &'static str)],
    theme_index: usize,

    prompt_action: PromptAction,
    prompt_height: usize,

    buffers: Buffers,
    windows: WindowTree<BufferViewId>,
    logger: Logger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct BufferViewId {
    buffer_id: BufferId,
    cursor_id: CursorId,
}

impl BufferViewId {
    fn new(buffer_id: BufferId, cursor_id: CursorId) -> Self {
        Self {
            buffer_id,
            cursor_id,
        }
    }
}

impl Display for BufferViewId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            formatter,
            "BufferViewId(buffer={}, cursor={})",
            self.buffer_id, self.cursor_id
        )
    }
}

impl Editor {
    #[inline]
    fn focus_on_buffer(&mut self, buffer_id: BufferId) {
        if self.windows.is_empty() {
            self.windows
                .add(BufferViewId::new(buffer_id, CursorId::default()));
        } else {
            self.windows
                .set_focused(BufferViewId::new(buffer_id, CursorId::default()));
        }
    }

    fn open_file(&mut self, file_path: PathBuf) -> Result<bool> {
        // Check if the buffer is already open
        if let Some(buffer_id) = self.buffers.find_by_path(&file_path) {
            self.focus_on_buffer(buffer_id);
            return Ok(false);
        }

        let (is_new_file, text) = if file_path.exists() {
            (
                false,
                Rope::from_reader(BufReader::new(File::open(&file_path)?))?,
            )
        } else {
            // Optimistically check if we can create it
            let is_new_file = File::open(&file_path)
                .map(|_| false)
                .or_else(|error| match error.kind() {
                    io::ErrorKind::NotFound => {
                        self.logger.info("[New file]".into());
                        Ok(true)
                    }
                    io::ErrorKind::PermissionDenied => {
                        self.logger.info(format!(
                            "Permission denied while opening {}",
                            file_path.display()
                        ));
                        Err(error)
                    }
                    _ => {
                        self.logger.info(format!(
                            "Could not open {} ({})",
                            file_path.display(),
                            error
                        ));
                        Err(error)
                    }
                })?;
            (is_new_file, Rope::new())
        };

        let repo = Repository::discover(&file_path).ok().map(RepositoryRc::new);

        // Store the new buffer
        let buffer_id = self.buffers.add(text, Some(file_path), repo);

        // Focus on the new buffer
        self.focus_on_buffer(buffer_id);

        Ok(is_new_file)
    }
}

impl Component for Editor {
    type Message = Message;
    type Properties = Rc<Context>;

    fn create(context: Rc<Context>, _frame: Rect, link: ComponentLink<Self>) -> Self {
        for file_path in context.args_files.iter().cloned() {
            link.send(Message::OpenFile(file_path));
        }
        let logger = Logger::new(link.clone());

        Self {
            context: Rc::clone(&context),
            link: link.clone(),
            themes: &THEMES,
            theme_index: 0,
            prompt_action: PromptAction::None,
            prompt_height: PROMPT_INACTIVE_HEIGHT,
            buffers: Buffers::new(context, link),
            windows: WindowTree::new(),
            logger,
        }
    }

    fn update(&mut self, message: Self::Message) -> ShouldRender {
        match message {
            Message::Cancel if self.prompt_action.is_interactive() => {
                self.prompt_action = PromptAction::Log {
                    message: "Cancel".into(),
                };
                self.prompt_height = self.prompt_action.initial_height();
            }
            Message::ChangeTheme => {
                self.theme_index = (self.theme_index + 1) % self.themes.len();
                if !self.prompt_action.is_interactive() {
                    self.logger.info(format!(
                        "Theme changed to {}",
                        self.themes[self.theme_index].1
                    ))
                }
            }
            Message::OpenFilePicker(source) if !self.prompt_action.is_interactive() => {
                self.prompt_action = PromptAction::OpenFile {
                    source,
                    on_open: self.link.callback(Message::OpenFile),
                    on_change_height: self.link.callback(Message::ChangePromptHeight),
                };
                self.prompt_height = self.prompt_action.initial_height();
            }
            Message::OpenFile(path) => {
                self.prompt_action = self.open_file(path).map_or_else(
                    |error| PromptAction::Log {
                        message: format!("Could not open file: {}", error.to_string()),
                    },
                    |new_file| {
                        if new_file {
                            PromptAction::Log {
                                message: "[New file]".into(),
                            }
                        } else {
                            PromptAction::None
                        }
                    },
                );
                self.prompt_height = self.prompt_action.initial_height();
            }
            Message::OpenBufferSwitcher if !self.prompt_action.is_interactive() => {
                self.prompt_action = PromptAction::SwitchBuffer {
                    entries: self
                        .buffers
                        .iter()
                        .map(|buffer| {
                            BufferEntry::new(
                                buffer.id(),
                                buffer.file_path().cloned(),
                                false,
                                buffer.content.len_bytes(),
                                buffer.mode,
                            )
                        })
                        .collect(),
                    on_select: self.link.callback(Message::SelectBuffer),
                    on_change_height: self.link.callback(Message::ChangePromptHeight),
                };
                self.prompt_height = self.prompt_action.initial_height();
            }
            Message::SelectBuffer(buffer_id) => {
                self.prompt_action = PromptAction::None;
                self.prompt_height = self.prompt_action.initial_height();
                self.focus_on_buffer(buffer_id);
            }
            Message::ChangePromptHeight(height) => {
                self.prompt_height = height;
            }
            Message::FocusNextComponent => self.windows.cycle_focus(CycleFocus::Next),
            Message::FocusPreviousComponent => self.windows.cycle_focus(CycleFocus::Previous),
            Message::SplitWindow(direction) if !self.buffers.is_empty() => {
                if let Some(view_id) = self.windows.get_focused() {
                    let buffer = self.buffers.get_mut(view_id.buffer_id).unwrap();
                    self.windows.insert_at_focused(
                        BufferViewId::new(
                            view_id.buffer_id,
                            buffer.duplicate_cursor(view_id.cursor_id),
                        ),
                        direction,
                    );
                }
            }
            Message::FullscreenWindow if !self.buffers.is_empty() => {
                self.windows.close_all_except_focused();
            }
            Message::ClosePane if !self.buffers.is_empty() => {
                self.windows.close_focused();
            }
            Message::Log(message) if !self.prompt_action.is_interactive() => {
                self.prompt_action = message
                    .map(|message| PromptAction::Log { message })
                    .unwrap_or(PromptAction::None);
                self.prompt_height = self.prompt_action.initial_height();
            }
            Message::Quit => {
                self.link.exit();
            }
            Message::Buffer(message) => self.buffers.handle_message(message),
            _ => {}
        }
        ShouldRender::Yes
    }

    fn view(&self) -> Layout {
        log::info!("Rendering");
        let buffers = if self.windows.is_empty() {
            Splash::item_with_key(
                FlexBasis::Auto,
                "splash",
                SplashProperties {
                    theme: Cow::Borrowed(&self.themes[self.theme_index].0.splash),
                },
            )
        } else {
            Item::auto(self.windows.layout(&mut |Window { id, focused, index }| {
                let buffer = self.buffers.get(id.buffer_id).unwrap();
                BufferView::with_key(
                    format!("{}.{}", index, id).as_str(),
                    BufferViewProperties {
                        context: self.context.clone(),
                        theme: Cow::Borrowed(&self.themes[self.theme_index].0.buffer),
                        focused: focused && !self.prompt_action.is_interactive(),
                        frame_id: index.one_based_index(),
                        mode: buffer.mode,
                        repo: buffer.repo.clone(),
                        content: buffer.content.clone(),
                        file_path: buffer.file_path().cloned(),
                        logger: self.logger.clone(),
                        cursor: BufferCursor::new(
                            id.buffer_id,
                            id.cursor_id,
                            buffer.cursor(id.cursor_id).clone(),
                            self.link.clone(),
                        ),
                        parse_tree: buffer.parse_tree().cloned(),
                        modified_status: buffer.modified_status(),
                    },
                )
            }))
        };

        Layout::column([
            buffers,
            Prompt::item_with_key(
                FlexBasis::Fixed(if self.prompt_action.is_none() {
                    PROMPT_INACTIVE_HEIGHT
                } else {
                    self.prompt_height
                }),
                "prompt",
                PromptProperties {
                    context: self.context.clone(),
                    theme: Cow::Borrowed(&self.themes[self.theme_index].0.prompt),
                    action: self.prompt_action.clone(),
                    // on_change: link.callback(Message::PromptStateChange),
                    // on_open_file: link.callback(Message::OpenFile),
                    // message: self.prompt_message.clone(),
                },
            ),
        ])
    }

    fn has_focus(&self) -> bool {
        true
    }

    fn input_binding(&self, pressed: &[Key]) -> BindingMatch<Self::Message> {
        let transition = BindingTransition::Clear;

        let message = match pressed {
            [Key::Ctrl('g')] => Message::Cancel,

            // Open a file
            [Key::Ctrl('x'), Key::Ctrl('f')] => Message::OpenFilePicker(FileSource::Directory),
            [Key::Ctrl('x'), Key::Ctrl('v')] => Message::OpenFilePicker(FileSource::Repository),

            // Buffer management
            [Key::Ctrl('x'), Key::Char('b') | Key::Ctrl('b')] => Message::OpenBufferSwitcher,
            [Key::Ctrl('x'), Key::Char('o') | Key::Ctrl('o')] => Message::FocusNextComponent,
            [Key::Ctrl('x'), Key::Char('i') | Key::Ctrl('i') | Key::Char('O') | Key::Ctrl('O')] => {
                Message::FocusPreviousComponent
            }
            // Window management
            [Key::Ctrl('x'), Key::Char('1') | Key::Ctrl('1')] => Message::FullscreenWindow,
            [Key::Ctrl('x'), Key::Char('2') | Key::Ctrl('2')] => {
                Message::SplitWindow(FlexDirection::Column)
            }
            [Key::Ctrl('x'), Key::Char('3') | Key::Ctrl('3')] => {
                Message::SplitWindow(FlexDirection::Row)
            }
            [Key::Ctrl('x'), Key::Char('0') | Key::Ctrl('0')] => Message::ClosePane,

            // Theme
            [Key::Ctrl('t')] => Message::ChangeTheme,

            // Quit
            [Key::Ctrl('x'), Key::Ctrl('c')] => Message::Quit,
            _ => {
                if let PromptAction::Log { .. } = self.prompt_action {
                    self.link.send(Message::Log(None));
                };
                return BindingMatch {
                    transition: BindingTransition::Continue,
                    message: Some(Self::Message::KeyPressed),
                };
            }
        };
        BindingMatch {
            transition,
            message: Some(message),
        }
    }
}

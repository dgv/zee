pub mod line_info;
pub mod status_bar;
pub mod textarea;

use std::{borrow::Cow, iter, path::PathBuf, rc::Rc};
use zi::{
    components::text::{Text, TextAlign, TextProperties},
    prelude::*,
};

use self::{
    line_info::{LineInfo, Properties as LineInfoProperties},
    status_bar::{Properties as StatusBarProperties, StatusBar, Theme as StatusBarTheme},
    textarea::{Properties as TextAreaProperties, TextArea},
};
use super::edit_tree_viewer::{
    EditTreeViewer, Properties as EditTreeViewerProperties, Theme as EditTreeViewerTheme,
};
use crate::{
    editor::{
        buffer::{BufferCursor, ModifiedStatus, RepositoryRc, DISABLE_TABS},
        Context, Logger,
    },
    mode::Mode,
    syntax::{highlight::Theme as SyntaxTheme, parse::ParseTree},
    undo::EditTree,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub border: Style,
    pub edit_tree_viewer: EditTreeViewerTheme,
    pub status_bar: StatusBarTheme,
    pub syntax: SyntaxTheme,
}

#[derive(Clone)]
pub struct Properties {
    pub context: Rc<Context>,
    pub logger: Logger,
    pub theme: Cow<'static, Theme>,
    pub focused: bool,
    pub frame_id: usize,
    pub mode: &'static Mode,
    pub repo: Option<RepositoryRc>,
    pub content: EditTree,
    pub file_path: Option<PathBuf>,
    pub cursor: BufferCursor,
    pub parse_tree: Option<ParseTree>,
    pub modified_status: ModifiedStatus,
}

impl PartialEq for Properties {
    fn eq(&self, other: &Self) -> bool {
        *self.theme == *other.theme
            && self.focused == other.focused
            && self.frame_id == other.frame_id
    }
}

#[derive(Debug)]
pub enum Message {
    // Movement
    Up,
    Down,
    Left,
    Right,
    CenterCursorVisually,

    // Editing
    ClearSelection,

    // Undo / Redo
    ToggleEditTree,
}

pub struct Buffer {
    properties: Properties,
    frame: Rect,
    line_offset: usize,
    viewing_edit_tree: bool,
}

impl Buffer {
    #[inline]
    fn reduce(&mut self, message: Message) {
        // Stateless
        match message {
            Message::CenterCursorVisually => self.center_visual_cursor(),

            // Message::Left if self.viewing_edit_tree => self.text.previous_child(),
            // Message::Right if self.viewing_edit_tree => self.text.next_child(),
            Message::ClearSelection if self.viewing_edit_tree => {
                self.viewing_edit_tree = false;
            }

            Message::ToggleEditTree => {
                self.viewing_edit_tree = !self.viewing_edit_tree;
            }

            // Message::Up if self.viewing_edit_tree => self
            //     .undo()
            //     .map(|diff| {
            //         undoing = true;
            //         diff
            //     })
            //     .unwrap_or_else(OpaqueDiff::empty),
            // Message::Down if self.viewing_edit_tree => self
            //     .redo()
            //     .map(|diff| {
            //         undoing = true;
            //         diff
            //     })
            //     .unwrap_or_else(OpaqueDiff::empty),
            _ => {}
        };
    }

    #[inline]
    fn ensure_cursor_in_view(&mut self) {
        let current_line = self
            .properties
            .content
            .char_to_line(self.properties.cursor.inner().range().start.0);
        let num_lines = self.frame.size.height.saturating_sub(1);
        if current_line < self.line_offset {
            self.line_offset = current_line;
        } else if current_line - self.line_offset > num_lines.saturating_sub(1) {
            self.line_offset = current_line + 1 - num_lines;
        }
    }

    fn center_visual_cursor(&mut self) {
        let line_index = self
            .properties
            .content
            .char_to_line(self.properties.cursor.inner().range().start.0);
        if line_index >= self.frame.size.height / 2
            && self.line_offset != line_index - self.frame.size.height / 2
        {
            self.line_offset = line_index - self.frame.size.height / 2;
        } else if self.line_offset != line_index {
            self.line_offset = line_index;
        } else {
            self.line_offset = 0;
        }
    }
}

impl Component for Buffer {
    type Properties = Properties;
    type Message = Message;

    fn create(properties: Self::Properties, frame: Rect, _link: ComponentLink<Self>) -> Self {
        let mut buffer = Self {
            line_offset: 0,
            viewing_edit_tree: false,

            properties,
            frame,
        };
        buffer.ensure_cursor_in_view();
        buffer
    }

    fn change(&mut self, properties: Self::Properties) -> ShouldRender {
        // let should_render = (self.properties.theme != properties.theme
        //     || self.properties.focused != properties.focused
        //     || self.properties.frame_id != properties.frame_id
        //     || self.properties.cursor.cursor != properties.cursor.cursor)
        //     .into();
        // should_render

        self.properties = properties;
        self.ensure_cursor_in_view();

        ShouldRender::Yes
    }

    fn resize(&mut self, frame: Rect) -> ShouldRender {
        self.frame = frame;
        self.ensure_cursor_in_view();
        ShouldRender::Yes
    }

    fn update(&mut self, message: Message) -> ShouldRender {
        self.reduce(message);
        ShouldRender::Yes
    }

    fn view(&self) -> Layout {
        // The textarea components that displays text
        let textarea = TextArea::with(TextAreaProperties {
            theme: self.properties.theme.syntax.clone(),
            focused: self.properties.focused,
            text: self.properties.content.staged().clone(),
            cursor: self.properties.cursor.inner().clone(),
            mode: self.properties.mode,
            line_offset: self.line_offset,
            parse_tree: self.properties.parse_tree.clone(),
        });

        // Vertical info bar which shows line specific diagnostics
        let line_info = LineInfo::with(LineInfoProperties {
            style: self.properties.theme.border,
            line_offset: self.line_offset,
            num_lines: self.properties.content.len_lines(),
        });

        // The "status bar" which shows information about the file etc.
        let status_bar = StatusBar::with(StatusBarProperties {
            current_line_index: self
                .properties
                .content
                .char_to_line(self.properties.cursor.inner().range().start.0),
            file_path: self.properties.file_path.clone(),
            focused: self.properties.focused,
            frame_id: self.properties.frame_id,
            modified_status: self.properties.modified_status,
            mode: self.properties.mode.into(),
            num_lines: self.properties.content.len_lines(),
            repository: self.properties.repo.clone(),
            size_bytes: self.properties.content.len_bytes() as u64,
            theme: self.properties.theme.status_bar.clone(),
            // TODO: Fix visual_cursor_x to display the column (i.e. unicode
            // width). It used to be computed by draw_line.
            visual_cursor_x: self.properties.cursor.inner().range().start.0,
        });

        // Edit-tree viewer (aka. undo/redo tree)
        let edit_tree_viewer = if self.viewing_edit_tree {
            Some(Item::fixed(EDIT_TREE_WIDTH)(Container::row([
                Item::fixed(1)(Text::with(
                    TextProperties::new().style(self.properties.theme.border),
                )),
                Item::auto(Container::column([
                    Item::auto(EditTreeViewer::with(EditTreeViewerProperties {
                        tree: self.properties.content.clone(),
                        theme: self.properties.theme.edit_tree_viewer.clone(),
                    })),
                    Item::fixed(1)(Text::with(
                        TextProperties::new()
                            .content("Edit Tree Viewer 🌴")
                            .style(self.properties.theme.border)
                            .align(TextAlign::Centre),
                    )),
                ])),
            ])))
        } else {
            None
        };

        Layout::column([
            Item::auto(Layout::row(
                iter::once(edit_tree_viewer)
                    .chain(iter::once(Some(Item::fixed(1)(line_info))))
                    .chain(iter::once(Some(Item::auto(textarea))))
                    .flatten(),
            )),
            Item::fixed(1)(status_bar),
        ])
    }

    fn has_focus(&self) -> bool {
        self.properties.focused
    }

    fn input_binding(&self, pressed: &[Key]) -> BindingMatch<Self::Message> {
        let mut transition = BindingTransition::Clear;
        log::debug!("{:?}", pressed);
        match pressed {
            // Cursor movement
            //
            // Up
            [Key::Ctrl('p')] | [Key::Up] if !self.viewing_edit_tree => {
                self.properties.cursor.move_up()
            }
            // Down
            [Key::Ctrl('n')] | [Key::Down] if !self.viewing_edit_tree => {
                self.properties.cursor.move_down()
            }
            // Left
            [Key::Ctrl('b')] | [Key::Left] if !self.viewing_edit_tree => {
                self.properties.cursor.move_left()
            }
            // Right
            [Key::Ctrl('f')] | [Key::Right] if !self.viewing_edit_tree => {
                self.properties.cursor.move_right()
            }
            // PageDown
            [Key::Ctrl('v')] | [Key::PageDown] => self
                .properties
                .cursor
                .move_down_n(self.frame.size.height.saturating_sub(1)),
            // PageUp
            [Key::Alt('v')] | [Key::PageUp] => self
                .properties
                .cursor
                .move_up_n(self.frame.size.height.saturating_sub(1)),
            // StartOfLine
            [Key::Ctrl('a')] | [Key::Home] => self.properties.cursor.move_to_start_of_line(),
            // EndOfLine
            [Key::Ctrl('e')] | [Key::End] => self.properties.cursor.move_to_end_of_line(),
            // StartOfBuffer
            [Key::Alt('<')] => self.properties.cursor.move_to_start_of_buffer(),
            // EndOfBuffer
            [Key::Alt('>')] => self.properties.cursor.move_to_end_of_buffer(),

            // Editing
            //
            // Delete forward
            [Key::Ctrl('d')] | [Key::Delete] => self.properties.cursor.delete_forward(),
            // Delete backward
            [Key::Backspace] => self.properties.cursor.delete_backward(),
            // Delete line
            [Key::Ctrl('k')] => self.properties.cursor.delete_line(),
            // Insert new line
            [Key::Char('\n')] => self.properties.cursor.insert_new_line(),
            // Insert tab
            [Key::Char('\t')] if DISABLE_TABS => self.properties.cursor.insert_tab(),
            // Insert character
            &[Key::Char(character)] if character != '\n' => {
                self.properties.cursor.insert_char(character)
            }

            // Selections
            //
            // Begin selection
            [Key::Null] | [Key::Ctrl(' ')] => self.properties.cursor.begin_selection(),
            // Clear selection
            [Key::Ctrl('g')] if !self.viewing_edit_tree => self.properties.cursor.clear_selection(),
            // Select all
            [Key::Ctrl('x'), Key::Char('h')] => self.properties.cursor.select_all(),
            // Copy selection to clipboard
            [Key::Alt('w')] => self.properties.cursor.copy_selection_to_clipboard(),
            // Cut selection to clipboard
            [Key::Ctrl('w')] => self.properties.cursor.cut_selection_to_clipboard(),
            // Paste from clipboard
            [Key::Ctrl('y')] => self.properties.cursor.paste_from_clipboard(),

            // Undo / Redo
            //
            // Undo
            [Key::Ctrl('_')] | [Key::Ctrl('z')] | [Key::Ctrl('/')] => self.properties.cursor.undo(),
            [Key::Ctrl('q')] => self.properties.cursor.redo(),

            // Buffer
            [Key::Ctrl('x'), Key::Ctrl('s')] | [Key::Ctrl('x'), Key::Char('s')] => {
                self.properties.cursor.save()
            }

            _ => {
                transition = BindingTransition::Continue;
            }
        };

        if transition == BindingTransition::Clear {
            return BindingMatch {
                transition,
                message: None,
            };
        }
        transition = BindingTransition::Clear;

        let message = match pressed {
            // Centre cursor visually
            [Key::Ctrl('l')] => Message::CenterCursorVisually,

            // View edit tree
            //
            // Toggle
            [Key::Ctrl('x'), Key::Char('u')] | [Key::Ctrl('x'), Key::Ctrl('u')] => {
                Message::ToggleEditTree
            }
            // Up
            [Key::Ctrl('p')] | [Key::Up] if self.viewing_edit_tree => Message::Up,
            // Down
            [Key::Ctrl('n')] | [Key::Down] if self.viewing_edit_tree => Message::Down,
            // Left
            [Key::Ctrl('b')] | [Key::Left] if self.viewing_edit_tree => Message::Left,
            // Right
            [Key::Ctrl('f')] | [Key::Right] if self.viewing_edit_tree => Message::Right,
            // Close
            [Key::Ctrl('g')] if self.viewing_edit_tree => Message::ClearSelection,

            [Key::Ctrl('x')] => {
                return {
                    BindingMatch {
                        transition: BindingTransition::Continue,
                        message: None,
                    }
                }
            }
            _ => {
                return {
                    BindingMatch {
                        transition: BindingTransition::Clear,
                        message: None,
                    }
                }
            }
        };

        BindingMatch {
            transition,
            message: Some(message),
        }
    }
}

const EDIT_TREE_WIDTH: usize = 36;

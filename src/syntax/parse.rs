use ropey::Rope;
use smallvec::SmallVec;
use std::{
    ops::{Deref, DerefMut, Range},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tree_sitter::{
    InputEdit as TreeSitterInputEdit, Language, Node, Parser, Point as TreeSitterPoint, Tree,
    TreeCursor,
};

use crate::{
    components::buffer::{Action, AsyncAction},
    error::{Error, Result},
    smallstring::SmallString,
    task::{Scheduler, TaskId},
};

pub struct ParserStatus {
    task_id: TaskId,
    parser: CancelableParser,
    parsed: Option<ParsedSyntax>, // None if the parsing operation has been cancelled
}

pub struct ParsedSyntax {
    tree: Tree,
    text: Rope,
}

pub struct SyntaxTree {
    language: Language,
    parsers: Vec<CancelableParser>,
    pub tree: Option<Tree>,
    current_parse_task: Option<(TaskId, CancelFlag)>,
}

impl SyntaxTree {
    pub fn new(language: Language) -> Self {
        Self {
            language,
            parsers: vec![],
            tree: None,
            current_parse_task: None,
        }
    }

    pub fn cursor(&self) -> Option<SyntaxCursor> {
        self.tree.as_ref().map(|tree| {
            let root_node = tree.root_node();
            SyntaxCursor {
                cursor: root_node.walk(),
                root: root_node,
            }
        })
    }

    pub fn ensure_tree(
        &mut self,
        scheduler: &mut Scheduler<Action>,
        tree_fn: impl FnOnce() -> Rope,
    ) -> Result<()> {
        match (self.tree.as_ref(), self.current_parse_task.as_ref()) {
            (None, None) => self.spawn_parse_task(scheduler, tree_fn(), true),
            _ => Ok(()),
        }
    }

    pub fn spawn_parse_task(
        &mut self,
        scheduler: &mut Scheduler<Action>,
        text: Rope,
        fresh: bool,
    ) -> Result<()> {
        let mut parser = self.parsers.pop().map(Ok).unwrap_or_else(|| -> Result<_> {
            let mut parser = Parser::new();
            parser
                .set_language(self.language)
                .map_err(Error::IncompatibleLanguageGrammar)?;
            Ok(CancelableParser::new(parser))
        })?;

        let cancel_flag = parser.cancel_flag().clone();
        let tree = self.tree.clone();
        let task_id = scheduler.spawn(move |task_id| {
            let maybe_tree = parser.parse_with(
                &mut |byte_index, _| {
                    let (chunk, chunk_byte_idx, _, _) = text.chunk_at_byte(byte_index);
                    assert!(byte_index >= chunk_byte_idx);

                    &chunk.as_bytes()[byte_index - chunk_byte_idx..]
                },
                if fresh { None } else { tree.as_ref() },
            );
            // Reset the parser for later reuse
            parser.reset();
            Action::Async(Ok(match maybe_tree {
                Some(tree) => AsyncAction::ParseSyntax(ParserStatus {
                    task_id,
                    parser,
                    parsed: Some(ParsedSyntax { tree, text }),
                }),
                None => AsyncAction::ParseSyntax(ParserStatus {
                    task_id,
                    parser,
                    parsed: None,
                }),
            }))
        })?;
        if let Some((_, old_cancel_flag)) = self.current_parse_task.as_ref() {
            old_cancel_flag.set();
        }
        self.current_parse_task = Some((task_id, cancel_flag));
        Ok(())
    }

    pub fn handle_parse_syntax_done(&mut self, status: ParserStatus) {
        let ParserStatus {
            task_id,
            parser,
            parsed,
        } = status;

        // Collect the parser for later reuse
        parser.cancel_flag().clear();
        self.parsers.push(parser);

        // If we weren't waiting for this task, return
        if self
            .current_parse_task
            .as_ref()
            .map(|(expected_task_id, _)| *expected_task_id != task_id)
            .unwrap_or(true)
        {
            return;
        }
        self.current_parse_task = None;

        // If the parser task hasn't been cancelled, store the new syntax tree
        if let Some(ParsedSyntax { tree, text }) = parsed {
            assert!(tree.root_node().end_byte() <= text.len_bytes());
            self.tree = Some(tree);
        }
    }

    pub fn edit(&mut self, diff: &OpaqueDiff) {
        match self.tree {
            Some(ref mut tree) if !diff.is_empty() => {
                tree.edit(&TreeSitterInputEdit {
                    start_byte: diff.byte_index,
                    old_end_byte: diff.byte_index + diff.old_length,
                    new_end_byte: diff.byte_index + diff.new_length,
                    // I don't use tree sitter's line/col tracking; I'm assuming
                    // here that passing in dummy values doesn't cause any other
                    // problem apart from incorrect line/col after editing a tree.
                    start_position: TreeSitterPoint::new(0, 0),
                    old_end_position: TreeSitterPoint::new(0, 0),
                    new_end_position: TreeSitterPoint::new(0, 0),
                });
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpaqueDiff {
    byte_index: usize,
    old_length: usize,
    new_length: usize,
}

impl OpaqueDiff {
    #[inline]
    pub fn new(byte_index: usize, old_length: usize, new_length: usize) -> Self {
        Self {
            byte_index,
            old_length,
            new_length,
        }
    }

    #[inline]
    pub fn empty() -> Self {
        Self {
            byte_index: 0,
            old_length: 0,
            new_length: 0,
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.byte_index == 0 && self.old_length == 0 && self.new_length == 0
    }

    #[inline]
    pub fn reverse(&self) -> Self {
        Self {
            byte_index: self.byte_index,
            old_length: self.new_length,
            new_length: self.old_length,
        }
    }
}

pub struct NodeTrace<T> {
    pub path: Vec<SmallString>,
    pub nth_children: SmallVec<[u16; 32]>,
    pub trace: SmallVec<[T; 32]>,
    pub is_error: bool,
    pub byte_range: Range<usize>,
}

impl<T> NodeTrace<T> {
    pub fn new() -> Self {
        Self {
            path: Vec::new(),
            nth_children: SmallVec::new(),
            trace: SmallVec::new(),
            is_error: false,
            byte_range: 0..0,
        }
    }

    pub fn clear(&mut self) {
        self.path.clear();
        self.nth_children.clear();
        self.trace.clear();
        self.is_error = false;
        self.byte_range = 0..0;
    }
}

pub struct SyntaxCursor<'a> {
    root: Node<'a>,
    cursor: TreeCursor<'a>,
}

impl<'a> SyntaxCursor<'a> {
    #[inline]
    pub fn trace_at<T>(
        &mut self,
        trace: &mut NodeTrace<T>,
        byte_index: usize,
        map_node: impl Fn(&Node<'a>) -> T,
    ) {
        // if trace.byte_range.contains(&byte_index) {
        //     return;
        // }

        self.cursor.reset(self.root);
        trace.clear();

        trace.is_error = trace.is_error || self.cursor.node().is_error();
        trace.path.push(self.cursor.node().kind().into());
        trace.trace.push(map_node(&self.cursor.node()));
        trace.nth_children.push(0);
        while let Some(nth_child) = self.cursor.goto_first_child_for_byte(byte_index) {
            trace.is_error = trace.is_error || self.cursor.node().is_error();
            trace.path.push(self.cursor.node().kind().into());
            trace.trace.push(map_node(&self.cursor.node()));
            trace.nth_children.push(nth_child as u16);
        }
        trace.trace.reverse();
        trace.nth_children.reverse();

        let node = self.cursor.node();
        trace.byte_range = node.start_byte()..node.end_byte();
    }
}

#[derive(Clone)]
struct CancelFlag(Arc<AtomicUsize>);

impl CancelFlag {
    fn set(&self) {
        self.0.store(CANCEL_FLAG_SET, Ordering::SeqCst);
    }

    fn clear(&self) {
        self.0.store(CANCEL_FLAG_UNSET, Ordering::SeqCst);
    }
}

struct CancelableParser {
    // `parser` should appear before `flag` as it holds a reference to the
    // cancellation flag and should be destroyed first
    parser: Parser,
    flag: CancelFlag,
}

impl CancelableParser {
    fn new(parser: Parser) -> Self {
        let flag = CancelFlag(Arc::new(AtomicUsize::new(CANCEL_FLAG_UNSET)));
        unsafe {
            parser.set_cancellation_flag(Some(&flag.0));
        }
        Self { flag, parser }
    }

    fn cancel_flag(&self) -> &CancelFlag {
        &self.flag
    }
}

impl Deref for CancelableParser {
    type Target = Parser;

    fn deref(&self) -> &Self::Target {
        &self.parser
    }
}

impl DerefMut for CancelableParser {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.parser
    }
}

const CANCEL_FLAG_UNSET: usize = 0;
const CANCEL_FLAG_SET: usize = 1;

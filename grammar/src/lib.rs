use lazy_static::lazy_static;
use tree_sitter::Language;

lazy_static! {
    pub static ref BASH: Language = unsafe { tree_sitter_bash() };
    pub static ref C: Language = unsafe { tree_sitter_c() };
    pub static ref CPP: Language = unsafe { tree_sitter_cpp() };
    pub static ref CSS: Language = unsafe { tree_sitter_css() };
    pub static ref GO: Language = unsafe { tree_sitter_go() };
    pub static ref HTML: Language = unsafe { tree_sitter_html() };
    pub static ref JAVASCRIPT: Language = unsafe { tree_sitter_javascript() };
    pub static ref JSON: Language = unsafe { tree_sitter_json() };
    pub static ref MARKDOWN: Language = unsafe { tree_sitter_markdown() };
    pub static ref PYTHON: Language = unsafe { tree_sitter_python() };
    pub static ref RUST: Language = unsafe { tree_sitter_rust() };
    pub static ref TYPESCRIPT: Language = unsafe { tree_sitter_typescript() };
    pub static ref TSX: Language = unsafe { tree_sitter_tsx() };
}
extern "C" {
    fn tree_sitter_bash() -> Language;
    fn tree_sitter_c() -> Language;
    fn tree_sitter_cpp() -> Language;
    fn tree_sitter_css() -> Language;
    fn tree_sitter_go() -> Language;
    fn tree_sitter_html() -> Language;
    fn tree_sitter_javascript() -> Language;
    fn tree_sitter_json() -> Language;
    fn tree_sitter_markdown() -> Language;
    fn tree_sitter_python() -> Language;
    fn tree_sitter_rust() -> Language;
    fn tree_sitter_typescript() -> Language;
    fn tree_sitter_tsx() -> Language;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instantiate_all_languages() {
        unsafe {
            tree_sitter_bash();
            tree_sitter_c();
            tree_sitter_cpp();
	    tree_sitter_css();
            tree_sitter_go();
            tree_sitter_html();
            tree_sitter_javascript();
            tree_sitter_json();
            tree_sitter_markdown();
            tree_sitter_python();
            tree_sitter_rust();
            tree_sitter_typescript();
            tree_sitter_tsx();
        }
    }
}

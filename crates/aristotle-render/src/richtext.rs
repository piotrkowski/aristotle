//! Small HTML fragments (dictionary definitions, footnotes) as styled
//! paragraphs a popup can set: blocks of runs that are plain, italic or
//! bold. Everything else in the markup (classes, links, tables, images) is
//! flattened or dropped; a popup on e-ink needs readable text, not layout.
//!
//! Two front ends feed one builder: [`from_html`] tokenises a fragment
//! (dictionary entries, which may not be well-formed XML), and
//! [`from_node`] walks a subtree of Plato's DOM (a footnote found in a book).

use plato_core::document::html::dom::{NodeData, NodeRef};
use plato_core::helpers::decode_entities;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub italic: bool,
    pub bold: bool,
}

/// A paragraph: a definition line, a list item, a note paragraph.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block {
    pub runs: Vec<Run>,
    /// Bold-ish heading line (a `<dt>`, `<h3>`).
    pub heading: bool,
}

impl Block {
    pub fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RichText {
    pub blocks: Vec<Block>,
}

impl RichText {
    /// One block per line of plain text.
    pub fn plain(text: &str) -> RichText {
        RichText {
            blocks: text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(|l| Block {
                    runs: vec![Run {
                        text: l.to_string(),
                        ..Run::default()
                    }],
                    heading: false,
                })
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// The text, blocks separated by newlines.
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(Block::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn len(&self) -> usize {
        self.blocks.iter().map(|b| b.text().len()).sum()
    }
}

fn is_block_tag(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "dd"
            | "dt"
            | "li"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "blockquote"
            | "section"
            | "aside"
            | "tr"
            | "br"
            | "ol"
            | "ul"
            | "dl"
            | "table"
            | "hr"
            | "header"
            | "footer"
            | "figcaption"
    )
}

fn is_skipped(name: &str) -> bool {
    matches!(
        name,
        "script" | "style" | "head" | "title" | "svg" | "math" | "rt" | "rp"
    )
}

#[derive(Default)]
struct Builder {
    out: RichText,
    cur: Block,
    italic: u32,
    bold: u32,
    skip: u32,
    /// Pending space between words, collapsed.
    space: bool,
}

impl Builder {
    fn text(&mut self, raw: &str) {
        if self.skip > 0 {
            return;
        }
        for (i, piece) in raw.split(char::is_whitespace).enumerate() {
            if i > 0 {
                self.space = true;
            }
            if piece.is_empty() {
                continue;
            }
            let lead = if self.space && !self.cur.runs.is_empty() {
                " "
            } else {
                ""
            };
            self.space = false;
            let (italic, bold) = (self.italic > 0, self.bold > 0);
            match self.cur.runs.last_mut() {
                Some(r) if r.italic == italic && r.bold == bold => {
                    r.text.push_str(lead);
                    r.text.push_str(piece);
                }
                _ => {
                    if !lead.is_empty() {
                        if let Some(r) = self.cur.runs.last_mut() {
                            r.text.push(' ');
                        }
                    }
                    self.cur.runs.push(Run {
                        text: piece.to_string(),
                        italic,
                        bold,
                    });
                }
            }
        }
    }

    fn end_block(&mut self) {
        self.space = false;
        let b = std::mem::take(&mut self.cur);
        if !b.runs.iter().all(|r| r.text.trim().is_empty()) {
            self.out.blocks.push(b);
        }
    }

    fn open(&mut self, name: &str) {
        if is_skipped(name) {
            self.skip += 1;
            return;
        }
        if is_block_tag(name) {
            self.end_block();
        }
        match name {
            "i" | "em" | "cite" | "var" | "dfn" => self.italic += 1,
            "b" | "strong" => self.bold += 1,
            "dt" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.bold += 1;
                self.cur.heading = true;
            }
            _ => {}
        }
    }

    fn close(&mut self, name: &str) {
        if is_skipped(name) {
            self.skip = self.skip.saturating_sub(1);
            return;
        }
        match name {
            "i" | "em" | "cite" | "var" | "dfn" => self.italic = self.italic.saturating_sub(1),
            "b" | "strong" | "dt" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.bold = self.bold.saturating_sub(1)
            }
            _ => {}
        }
        if is_block_tag(name) {
            self.end_block();
        }
    }

    fn finish(mut self) -> RichText {
        self.end_block();
        self.out
    }
}

/// Convert an HTML fragment. Tolerant: unclosed and void tags, comments and
/// stray `<` are all fine.
pub fn from_html(html: &str) -> RichText {
    let mut b = Builder::default();
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        b.text(&decode_entities(&rest[..lt]));
        rest = &rest[lt..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map_or("", |e| &rest[e + 3..]);
            continue;
        }
        let Some(gt) = rest.find('>') else {
            b.text(&decode_entities(rest));
            rest = "";
            break;
        };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];
        let closing = tag.starts_with('/');
        let self_closing = tag.ends_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        if closing {
            b.close(&name);
        } else {
            b.open(&name);
            if self_closing || matches!(name.as_str(), "br" | "hr" | "img") {
                b.close(&name);
            }
        }
    }
    b.text(&decode_entities(rest));
    b.finish()
}

/// Convert a subtree of Plato's DOM, the node itself included.
pub fn from_node(node: NodeRef) -> RichText {
    let mut b = Builder::default();
    walk(node, &mut b);
    b.finish()
}

fn walk(node: NodeRef, b: &mut Builder) {
    match node.data() {
        NodeData::Text(t) | NodeData::Whitespace(t) => b.text(&decode_entities(&t.text)),
        NodeData::Element(e) => {
            let name = e.name.to_ascii_lowercase();
            b.open(&name);
            for c in node.children() {
                walk(c, b);
            }
            b.close(&name);
        }
        NodeData::Root | NodeData::Wrapper(_) => {
            for c in node.children() {
                walk(c, b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wiktionary_entry() {
        let html = "<section class=\"mw-parser-output\" lang=\"pl\"><p><i>rzeczownik, rodzaj żeński</i></p><dl><dd>(1.1) <a href=\"bword://oddech\">oddech</a>, <a>tchnienie</a></dd>\n<dd>(1.2) dusza &amp; duch</dd></dl><dl><dt><span>etymologia<span>:</span></span></dt><dd>gr. ψύχω</dd></dl></section>";
        let r = from_html(html);
        let lines: Vec<String> = r.blocks.iter().map(Block::text).collect();
        assert_eq!(
            lines,
            [
                "rzeczownik, rodzaj żeński",
                "(1.1) oddech, tchnienie",
                "(1.2) dusza & duch",
                "etymologia:",
                "gr. ψύχω"
            ]
        );
        assert!(r.blocks[0].runs[0].italic);
        assert!(r.blocks[3].heading && r.blocks[3].runs[0].bold);
    }

    #[test]
    fn whitespace_and_styles() {
        let r = from_html("a  <i>b\n c</i>d<br>e<!-- x -->f <b>g");
        assert_eq!(r.blocks.len(), 2);
        assert_eq!(r.blocks[0].text(), "a b cd");
        assert_eq!(r.blocks[1].text(), "ef g");
        assert_eq!(r.blocks[0].runs[1].text, "b c");
        assert!(r.blocks[0].runs[1].italic);
        assert!(r.blocks[1].runs.last().unwrap().bold);
    }
}

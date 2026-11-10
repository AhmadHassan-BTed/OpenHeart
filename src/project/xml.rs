//! Minimal XML reader for Maven POMs and MSBuild project files: elements, attributes, text,
//! CDATA and the five predefined entities. Comments, processing instructions and the
//! document type declaration are skipped. Element names drop their namespace prefix. Every
//! element keeps the 1-based line of its start tag.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct XmlElement {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XmlElement>,
    pub text: String,
    pub line: u32,
}

impl XmlElement {
    pub fn child(&self, name: &str) -> Option<&XmlElement> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'s>(&'s self, name: &'s str) -> impl Iterator<Item = &'s XmlElement> + 's {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Trimmed text of a direct child (`None` when absent or empty).
    pub fn child_text(&self, name: &str) -> Option<String> {
        self.child(name)
            .map(|c| c.text.trim().to_string())
            .filter(|t| !t.is_empty())
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Every descendant (depth first, document order) named `name`.
    pub fn descendants_named<'s>(&'s self, name: &str, out: &mut Vec<&'s XmlElement>) {
        for c in &self.children {
            if c.name == name {
                out.push(c);
            }
            c.descendants_named(name, out);
        }
    }
}

fn local_name(name: &str) -> String {
    match name.rfind(':') {
        Some(i) => name[i + 1..].to_string(),
        None => name.to_string(),
    }
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Parses a document and returns its root element (`None` when there is none).
pub fn parse(source: &str) -> Option<XmlElement> {
    let bytes = source.as_bytes();
    let mut line_starts = vec![0usize];
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    let line_of = |pos: usize| -> u32 {
        match line_starts.binary_search(&pos) {
            Ok(i) => (i + 1) as u32,
            Err(i) => i as u32,
        }
    };

    let mut stack: Vec<XmlElement> = Vec::new();
    let mut root: Option<XmlElement> = None;
    let mut i = 0usize;
    let n = bytes.len();
    while i < n {
        if bytes[i] != b'<' {
            let end = source[i..].find('<').map(|p| i + p).unwrap_or(n);
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&decode_entities(&source[i..end]));
            }
            i = end;
            continue;
        }
        let rest = &source[i..];
        if rest.starts_with("<!--") {
            i = rest.find("-->").map(|p| i + p + 3).unwrap_or(n);
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>").map(|p| i + p).unwrap_or(n);
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&source[(i + 9).min(end)..end]);
            }
            i = (end + 3).min(n);
            continue;
        }
        if rest.starts_with("<?") {
            i = rest.find("?>").map(|p| i + p + 2).unwrap_or(n);
            continue;
        }
        if rest.starts_with("<!") {
            i = rest.find('>').map(|p| i + p + 1).unwrap_or(n);
            continue;
        }
        if rest.starts_with("</") {
            i = rest.find('>').map(|p| i + p + 1).unwrap_or(n);
            if let Some(done) = stack.pop() {
                match stack.last_mut() {
                    Some(parent) => parent.children.push(done),
                    None => {
                        if root.is_none() {
                            root = Some(done);
                        }
                    }
                }
            }
            continue;
        }
        // Start tag: find its end outside attribute quotes.
        let mut j = i + 1;
        let mut quote: Option<u8> = None;
        while j < n {
            let b = bytes[j];
            match quote {
                Some(q) if b == q => quote = None,
                Some(_) => {}
                None if b == b'"' || b == b'\'' => quote = Some(b),
                None if b == b'>' => break,
                None => {}
            }
            j += 1;
        }
        let tag = &source[i + 1..j.min(n)];
        let self_closing = tag.ends_with('/');
        let tag = tag.trim_end_matches('/');
        let element = parse_tag(tag, line_of(i));
        i = (j + 1).min(n);
        if self_closing {
            match stack.last_mut() {
                Some(parent) => parent.children.push(element),
                None => {
                    if root.is_none() {
                        root = Some(element);
                    }
                }
            }
        } else {
            stack.push(element);
        }
    }
    // Unclosed elements: attach what was read.
    while let Some(done) = stack.pop() {
        match stack.last_mut() {
            Some(parent) => parent.children.push(done),
            None => {
                if root.is_none() {
                    root = Some(done);
                }
            }
        }
    }
    root
}

fn parse_tag(tag: &str, line: u32) -> XmlElement {
    let tag = tag.trim();
    let name_end = tag
        .find(|c: char| c.is_whitespace())
        .unwrap_or(tag.len());
    let mut element = XmlElement {
        name: local_name(&tag[..name_end]),
        line,
        ..Default::default()
    };
    let mut rest = tag[name_end..].trim_start();
    while !rest.is_empty() {
        let eq = match rest.find('=') {
            Some(e) => e,
            None => break,
        };
        let key = rest[..eq].trim().to_string();
        let after = rest[eq + 1..].trim_start();
        let quote = match after.chars().next() {
            Some(q) if q == '"' || q == '\'' => q,
            _ => break,
        };
        let close = match after[1..].find(quote) {
            Some(c) => c + 1,
            None => break,
        };
        element
            .attrs
            .push((local_name(&key), decode_entities(&after[1..close])));
        rest = after[close + 1..].trim_start();
    }
    element
}

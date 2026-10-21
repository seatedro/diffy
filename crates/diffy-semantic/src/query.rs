pub(crate) struct Pattern<'q> {
    pub(crate) start: usize,
    pub(crate) captures: Vec<Capture<'q>>,
    pub(crate) predicated: bool,
}

pub(crate) struct Capture<'q> {
    pub(crate) name: &'q str,
    pub(crate) kinds: Vec<(String, bool)>,
    pub(crate) root: bool,
}

enum Token<'q> {
    Open,
    Close,
    OpenAlt,
    CloseAlt,
    Str(String),
    Word(&'q str),
    Field,
    Capture(&'q str),
    Predicate,
    Other,
}

pub(crate) fn patterns(src: &str) -> Vec<Pattern<'_>> {
    let mut reader = Reader {
        src,
        at: 0,
        peeked: None,
    };
    let mut out = Vec::new();
    while let Some((start, token)) = reader.next() {
        let mut pattern = Pattern {
            start,
            captures: Vec::new(),
            predicated: false,
        };
        reader.pattern(token, 0, &mut pattern);
        out.push(pattern);
    }
    out
}

struct Reader<'q> {
    src: &'q str,
    at: usize,
    peeked: Option<(usize, Token<'q>)>,
}

impl<'q> Reader<'q> {
    fn pattern(
        &mut self,
        token: Token<'q>,
        depth: usize,
        out: &mut Pattern<'q>,
    ) -> Vec<(String, bool)> {
        let kinds = match token {
            Token::Open => self.group(depth, out),
            Token::OpenAlt => {
                let mut kinds = Vec::new();
                while let Some((_, token)) = self.next() {
                    if matches!(token, Token::CloseAlt) {
                        break;
                    }
                    kinds.extend(self.pattern(token, depth, out));
                }
                kinds
            }
            Token::Str(text) => vec![(text, false)],
            _ => Vec::new(),
        };
        loop {
            match self.peek() {
                Some(Token::Other) => {
                    self.next();
                }
                Some(&Token::Capture(name)) => {
                    self.next();
                    out.captures.push(Capture {
                        name,
                        kinds: kinds.clone(),
                        root: depth == 0,
                    });
                }
                _ => return kinds,
            }
        }
    }

    fn group(&mut self, depth: usize, out: &mut Pattern<'q>) -> Vec<(String, bool)> {
        match self.next() {
            Some((_, Token::Word(word))) => {
                self.children(depth + 1, out);
                match word.rsplit('/').next() {
                    Some("_") | None => Vec::new(),
                    Some(kind) => vec![(kind.to_owned(), true)],
                }
            }
            Some((_, Token::Predicate)) => {
                out.predicated = true;
                self.skip();
                Vec::new()
            }
            Some((_, Token::Close)) | None => Vec::new(),
            Some((_, token)) => {
                self.pattern(token, depth, out);
                self.children(depth, out);
                Vec::new()
            }
        }
    }

    fn children(&mut self, depth: usize, out: &mut Pattern<'q>) {
        while let Some((_, token)) = self.next() {
            match token {
                Token::Close => return,
                Token::Field => {
                    if let Some((_, token)) = self.next() {
                        self.pattern(token, depth, out);
                    }
                }
                Token::Other => {}
                token => {
                    self.pattern(token, depth, out);
                }
            }
        }
    }

    fn skip(&mut self) {
        let mut depth = 0;
        while let Some((_, token)) = self.next() {
            match token {
                Token::Open => depth += 1,
                Token::Close if depth == 0 => return,
                Token::Close => depth -= 1,
                _ => {}
            }
        }
    }

    fn peek(&mut self) -> Option<&Token<'q>> {
        if self.peeked.is_none() {
            self.peeked = self.read();
        }
        self.peeked.as_ref().map(|(_, token)| token)
    }

    fn next(&mut self) -> Option<(usize, Token<'q>)> {
        self.peeked.take().or_else(|| self.read())
    }

    fn read(&mut self) -> Option<(usize, Token<'q>)> {
        let bytes = self.src.as_bytes();
        loop {
            match bytes.get(self.at)? {
                b';' => {
                    while bytes.get(self.at).is_some_and(|&b| b != b'\n') {
                        self.at += 1;
                    }
                }
                b if b.is_ascii_whitespace() => self.at += 1,
                _ => break,
            }
        }
        let start = self.at;
        self.at += 1;
        let token = match bytes[start] {
            b'(' => Token::Open,
            b')' => Token::Close,
            b'[' => Token::OpenAlt,
            b']' => Token::CloseAlt,
            b'"' => Token::Str(self.string()),
            b'@' => Token::Capture(self.name()),
            b'#' => {
                self.name();
                Token::Predicate
            }
            b'!' => {
                self.word();
                Token::Other
            }
            b if is_word(b) => {
                self.at = start;
                let word = self.word();
                if bytes.get(self.at) == Some(&b':') {
                    self.at += 1;
                    Token::Field
                } else {
                    Token::Word(word)
                }
            }
            _ => Token::Other,
        };
        Some((start, token))
    }

    fn word(&mut self) -> &'q str {
        self.take(is_word)
    }

    fn name(&mut self) -> &'q str {
        self.take(|b| is_word(b) || matches!(b, b'.' | b'?' | b'!'))
    }

    fn take(&mut self, accept: impl Fn(u8) -> bool) -> &'q str {
        let start = self.at;
        while self.src.as_bytes().get(self.at).is_some_and(|&b| accept(b)) {
            self.at += 1;
        }
        &self.src[start..self.at]
    }

    fn string(&mut self) -> String {
        let mut out = String::new();
        let mut chars = self.src[self.at..].char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => {
                    self.at += i + 1;
                    return out;
                }
                '\\' => match chars.next() {
                    Some((_, 'n')) => out.push('\n'),
                    Some((_, 't')) => out.push('\t'),
                    Some((_, 'r')) => out.push('\r'),
                    Some((_, '0')) => out.push('\0'),
                    Some((_, c)) => out.push(c),
                    None => {}
                },
                c => out.push(c),
            }
        }
        self.at = self.src.len();
        out
    }
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'/')
}

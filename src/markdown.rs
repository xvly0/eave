
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MdInlineSpan {
    /// Start offset in UTF-16 code units inside `MdRichText::plain`.
    pub start_u16: u32,
    /// Length in UTF-16 code units inside `MdRichText::plain`.
    pub len_u16: u32,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub math: bool,
    pub link_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MdRichText {
    pub plain: String,
    pub spans: Vec<MdInlineSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MdBlock {
    Heading {
        level: u8,
        text: MdRichText,
    },
    Paragraph(MdRichText),
    CodeBlock {
        lang: String,
        code: String,
    },
    MathBlock(String),
    BulletItem {
        indent: u8,
        text: MdRichText,
    },
    NumberedItem {
        indent: u8,
        number: String,
        text: MdRichText,
    },
    Blockquote(MdRichText),
    HorizontalRule,
    Table {
        headers: Vec<MdRichText>,
        rows: Vec<Vec<MdRichText>>,
    },
}

pub fn parse_markdown(input: &str) -> Vec<MdBlock> {
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut blocks = Vec::new();
    let mut i = 0usize;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let fence_ch = trimmed.as_bytes()[0];
            let fence_len = trimmed.bytes().take_while(|&b| b == fence_ch).count();
            let lang = trimmed[fence_len..].trim().to_string();
            i += 1;
            let mut code_lines = Vec::new();
            while i < lines.len() {
                let t = lines[i].trim();
                let closing_len = t.bytes().take_while(|&b| b == fence_ch).count();
                if closing_len >= fence_len && t[closing_len..].trim().is_empty() {
                    i += 1;
                    break;
                }
                code_lines.push(lines[i]);
                i += 1;
            }
            blocks.push(MdBlock::CodeBlock {
                lang,
                code: code_lines.join("\n"),
            });
            continue;
        }

        if trimmed.starts_with("$$") {
            let after = &trimmed[2..];
            if let Some(end_idx) = after.find("$$") {
                let expr = after[..end_idx].trim();
                if !expr.is_empty() {
                    blocks.push(MdBlock::MathBlock(latex_to_unicode(expr)));
                }
                i += 1;
                continue;
            }
            let mut math_lines = Vec::new();
            if !after.trim().is_empty() {
                math_lines.push(after.trim());
            }
            i += 1;
            while i < lines.len() {
                let t = lines[i].trim();
                if let Some(before) = t.strip_suffix("$$") {
                    if !before.trim().is_empty() {
                        math_lines.push(before.trim());
                    }
                    i += 1;
                    break;
                }
                if t == "$$" {
                    i += 1;
                    break;
                }
                math_lines.push(t);
                i += 1;
            }
            let expr = math_lines.join(" ");
            if !expr.trim().is_empty() {
                blocks.push(MdBlock::MathBlock(latex_to_unicode(&expr)));
            }
            continue;
        }
        if trimmed.starts_with("\\[") {
            let after = &trimmed[2..];
            if let Some(end_idx) = after.find("\\]") {
                let expr = after[..end_idx].trim();
                if !expr.is_empty() {
                    blocks.push(MdBlock::MathBlock(latex_to_unicode(expr)));
                }
                i += 1;
                continue;
            }
            let mut math_lines = Vec::new();
            if !after.trim().is_empty() {
                math_lines.push(after.trim());
            }
            i += 1;
            while i < lines.len() {
                let t = lines[i].trim();
                if let Some(before) = t.strip_suffix("\\]") {
                    if !before.trim().is_empty() {
                        math_lines.push(before.trim());
                    }
                    i += 1;
                    break;
                }
                if t == "\\]" {
                    i += 1;
                    break;
                }
                math_lines.push(t);
                i += 1;
            }
            let expr = math_lines.join(" ");
            if !expr.trim().is_empty() {
                blocks.push(MdBlock::MathBlock(latex_to_unicode(&expr)));
            }
            continue;
        }

        if is_horizontal_rule(trimmed) {
            blocks.push(MdBlock::HorizontalRule);
            i += 1;
            continue;
        }

        if let Some((level, rest)) = parse_heading(trimmed) {
            blocks.push(MdBlock::Heading {
                level,
                text: parse_inline(rest),
            });
            i += 1;
            continue;
        }

        if i + 1 < lines.len() && is_table_header_pair(line, lines[i + 1]) {
            let headers = split_table_cells(line)
                .into_iter()
                .map(|c| parse_inline(c.trim()))
                .collect::<Vec<_>>();
            i += 2;
            let mut rows = Vec::new();
            while i < lines.len() {
                let row_line = lines[i].trim();
                if row_line.is_empty() || !row_line.contains('|') {
                    break;
                }
                let mut cells = split_table_cells(lines[i])
                    .into_iter()
                    .map(|c| parse_inline(c.trim()))
                    .collect::<Vec<_>>();
                while cells.len() < headers.len() {
                    cells.push(MdRichText::default());
                }
                cells.truncate(headers.len());
                rows.push(cells);
                i += 1;
            }
            blocks.push(MdBlock::Table { headers, rows });
            continue;
        }

        if trimmed.starts_with('>') {
            let mut quote_lines = Vec::new();
            while i < lines.len() {
                let t = lines[i].trim();
                if let Some(rest) = t.strip_prefix('>') {
                    quote_lines.push(rest.strip_prefix(' ').unwrap_or(rest));
                    i += 1;
                } else {
                    break;
                }
            }
            blocks.push(MdBlock::Blockquote(parse_inline(&quote_lines.join("\n"))));
            continue;
        }

        if let Some((indent, rest)) = parse_bullet_item(line) {
            blocks.push(MdBlock::BulletItem {
                indent,
                text: parse_inline(rest),
            });
            i += 1;
            continue;
        }

        if let Some((indent, number, rest)) = parse_numbered_item(line) {
            blocks.push(MdBlock::NumberedItem {
                indent,
                number,
                text: parse_inline(rest),
            });
            i += 1;
            continue;
        }

        let mut para_lines = Vec::new();
        while i < lines.len() {
            let cur = lines[i];
            let cur_t = cur.trim();
            if cur_t.is_empty()
                || cur_t.starts_with("```")
                || cur_t.starts_with("~~~")
                || cur_t.starts_with("$$")
                || cur_t.starts_with("\\[")
                || is_horizontal_rule(cur_t)
                || parse_heading(cur_t).is_some()
                || cur_t.starts_with('>')
                || parse_bullet_item(cur).is_some()
                || parse_numbered_item(cur).is_some()
                || (i + 1 < lines.len() && is_table_header_pair(cur, lines[i + 1]))
            {
                break;
            }
            para_lines.push(cur.trim_end());
            i += 1;
        }
        if !para_lines.is_empty() {
            blocks.push(MdBlock::Paragraph(parse_inline(&para_lines.join("\n"))));
        }
    }

    blocks
}

fn is_horizontal_rule(trimmed: &str) -> bool {
    let bytes: Vec<u8> = trimmed.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if bytes.len() < 3 {
        return false;
    }
    let ch = bytes[0];
    (ch == b'-' || ch == b'*' || ch == b'_') && bytes.iter().all(|&b| b == ch)
}

fn parse_heading(trimmed: &str) -> Option<(u8, &str)> {
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&hashes) {
        let rest = &trimmed[hashes..];
        if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
            return Some((hashes as u8, rest.trim()));
        }
    }
    None
}

fn parse_bullet_item(line: &str) -> Option<(u8, &str)> {
    let leading_spaces = line.bytes().take_while(|&b| b == b' ' || b == b'\t').count();
    let rest = &line[leading_spaces..];
    let first = *rest.as_bytes().first()?;
    if (first == b'-' || first == b'*' || first == b'+')
        && rest.as_bytes().get(1) == Some(&b' ')
    {
        let indent = ((leading_spaces / 2) as u8).min(3);
        return Some((indent, rest[2..].trim_start()));
    }
    None
}

fn parse_numbered_item(line: &str) -> Option<(u8, String, &str)> {
    let leading_spaces = line.bytes().take_while(|&b| b == b' ' || b == b'\t').count();
    let rest = &line[leading_spaces..];
    let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || digits > 4 {
        return None;
    }
    let after = &rest[digits..];
    if (after.starts_with(". ") || after.starts_with(") ")) && after.len() >= 2 {
        let indent = ((leading_spaces / 2) as u8).min(3);
        let num = format!("{}.", &rest[..digits]);
        return Some((indent, num, after[2..].trim_start()));
    }
    None
}

fn is_table_header_pair(header_line: &str, sep_line: &str) -> bool {
    if !header_line.contains('|') || !sep_line.contains('|') {
        return false;
    }
    let cells = split_table_cells(sep_line);
    if cells.is_empty() {
        return false;
    }
    cells.iter().all(|c| {
        let t = c.trim();
        !t.is_empty()
            && t.bytes().all(|b| b == b'-' || b == b':' || b == b' ')
            && t.contains('-')
    })
}

fn split_table_cells(line: &str) -> Vec<&str> {
    let mut s = line.trim();
    if let Some(stripped) = s.strip_prefix('|') {
        s = stripped;
    }
    if let Some(stripped) = s.strip_suffix('|') {
        s = stripped;
    }
    s.split('|').collect()
}

/// into plain text + UTF-16 `MdInlineSpan`s.
pub fn parse_inline(src: &str) -> MdRichText {
    let mut plain = String::with_capacity(src.len());
    let mut spans = Vec::new();
    let mut u16_pos = 0u32;
    let bytes = src.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if src[i..].starts_with("\\(") {
            if let Some(rel_end) = src[i + 2..].find("\\)") {
                let raw_expr = &src[i + 2..i + 2 + rel_end];
                let rendered = latex_to_unicode(raw_expr);
                let start = u16_pos;
                let len = rendered.encode_utf16().count() as u32;
                plain.push_str(&rendered);
                u16_pos += len;
                if len > 0 {
                    spans.push(MdInlineSpan {
                        start_u16: start,
                        len_u16: len,
                        bold: false,
                        italic: false,
                        strike: false,
                        code: false,
                        math: true,
                        link_url: None,
                    });
                }
                i = i + 2 + rel_end + 2;
                continue;
            }
        }
        if src[i..].starts_with("\\[") {
            if let Some(rel_end) = src[i + 2..].find("\\]") {
                let raw_expr = &src[i + 2..i + 2 + rel_end];
                let rendered = latex_to_unicode(raw_expr);
                let start = u16_pos;
                let len = rendered.encode_utf16().count() as u32;
                plain.push_str(&rendered);
                u16_pos += len;
                if len > 0 {
                    spans.push(MdInlineSpan {
                        start_u16: start,
                        len_u16: len,
                        bold: false,
                        italic: false,
                        strike: false,
                        code: false,
                        math: true,
                        link_url: None,
                    });
                }
                i = i + 2 + rel_end + 2;
                continue;
            }
        }

        if src[i..].starts_with("$$") {
            if let Some(rel_end) = src[i + 2..].find("$$") {
                if rel_end > 0 {
                    let raw_expr = &src[i + 2..i + 2 + rel_end];
                    let rendered = latex_to_unicode(raw_expr);
                    let start = u16_pos;
                    let len = rendered.encode_utf16().count() as u32;
                    plain.push_str(&rendered);
                    u16_pos += len;
                    if len > 0 {
                        spans.push(MdInlineSpan {
                            start_u16: start,
                            len_u16: len,
                            bold: false,
                            italic: false,
                            strike: false,
                            code: false,
                            math: true,
                            link_url: None,
                        });
                    }
                    i = i + 2 + rel_end + 2;
                    continue;
                }
            }
        }
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] != b' ' && bytes[i + 1] != b'$' {
            if let Some(rel_end) = find_closing_single_dollar(&src[i + 1..]) {
                let raw_expr = &src[i + 1..i + 1 + rel_end];
                if !raw_expr.ends_with(' ') && !raw_expr.contains('\n') {
                    let rendered = latex_to_unicode(raw_expr);
                    let start = u16_pos;
                    let len = rendered.encode_utf16().count() as u32;
                    plain.push_str(&rendered);
                    u16_pos += len;
                    if len > 0 {
                        spans.push(MdInlineSpan {
                            start_u16: start,
                            len_u16: len,
                            bold: false,
                            italic: false,
                            strike: false,
                            code: false,
                            math: true,
                            link_url: None,
                        });
                    }
                    i = i + 1 + rel_end + 1;
                    continue;
                }
            }
        }

        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_punctuation() {
            let ch = bytes[i + 1] as char;
            plain.push(ch);
            u16_pos += ch.len_utf16() as u32;
            i += 2;
            continue;
        }

        if bytes[i] == b'`' {
            if let Some(rel_end) = src[i + 1..].find('`') {
                let inner = &src[i + 1..i + 1 + rel_end];
                let start = u16_pos;
                let len = inner.encode_utf16().count() as u32;
                plain.push_str(inner);
                u16_pos += len;
                if len > 0 {
                    spans.push(MdInlineSpan {
                        start_u16: start,
                        len_u16: len,
                        bold: false,
                        italic: false,
                        strike: false,
                        code: true,
                        math: false,
                        link_url: None,
                    });
                }
                i = i + 1 + rel_end + 1;
                continue;
            }
        }

        if bytes[i] == b'[' {
            if let Some(rel_rb) = src[i + 1..].find("](") {
                let after_lp = i + 1 + rel_rb + 2;
                if let Some(rel_rp) = src[after_lp..].find(')') {
                    let label = &src[i + 1..i + 1 + rel_rb];
                    let url = src[after_lp..after_lp + rel_rp].trim().to_string();
                    let start = u16_pos;
                    let len = label.encode_utf16().count() as u32;
                    plain.push_str(label);
                    u16_pos += len;
                    if len > 0 && !url.is_empty() {
                        spans.push(MdInlineSpan {
                            start_u16: start,
                            len_u16: len,
                            bold: false,
                            italic: false,
                            strike: false,
                            code: false,
                            math: false,
                            link_url: Some(url),
                        });
                    }
                    i = after_lp + rel_rp + 1;
                    continue;
                }
            }
        }

        if src[i..].starts_with("https://") || src[i..].starts_with("http://") {
            let mut end = i;
            while end < bytes.len()
                && !bytes[end].is_ascii_whitespace()
                && !matches!(bytes[end], b'<' | b'>' | b'"' | b'\'')
            {
                end += 1;
            }
            while end > i
                && matches!(
                    bytes[end - 1],
                    b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b']'
                )
            {
                end -= 1;
            }
            let url = &src[i..end];
            if url.len() > 8 {
                let start = u16_pos;
                let len = url.encode_utf16().count() as u32;
                plain.push_str(url);
                u16_pos += len;
                spans.push(MdInlineSpan {
                    start_u16: start,
                    len_u16: len,
                    bold: false,
                    italic: false,
                    strike: false,
                    code: false,
                    math: false,
                    link_url: Some(url.to_string()),
                });
                i = end;
                continue;
            }
        }

        if src[i..].starts_with("***") {
            if let Some(rel_end) = src[i + 3..].find("***") {
                if rel_end > 0 {
                    let inner = &src[i + 3..i + 3 + rel_end];
                    let start = u16_pos;
                    let len = inner.encode_utf16().count() as u32;
                    plain.push_str(inner);
                    u16_pos += len;
                    spans.push(MdInlineSpan {
                        start_u16: start,
                        len_u16: len,
                        bold: true,
                        italic: true,
                        strike: false,
                        code: false,
                        math: false,
                        link_url: None,
                    });
                    i = i + 3 + rel_end + 3;
                    continue;
                }
            }
        }

        if src[i..].starts_with("**") || src[i..].starts_with("__") {
            let marker = &src[i..i + 2];
            if let Some(rel_end) = src[i + 2..].find(marker) {
                if rel_end > 0 {
                    let inner_parsed = parse_inline(&src[i + 2..i + 2 + rel_end]);
                    let start = u16_pos;
                    let len = inner_parsed.plain.encode_utf16().count() as u32;
                    plain.push_str(&inner_parsed.plain);
                    u16_pos += len;
                    spans.push(MdInlineSpan {
                        start_u16: start,
                        len_u16: len,
                        bold: true,
                        italic: false,
                        strike: false,
                        code: false,
                        math: false,
                        link_url: None,
                    });
                    for mut sp in inner_parsed.spans {
                        sp.start_u16 += start;
                        sp.bold = true;
                        spans.push(sp);
                    }
                    i = i + 2 + rel_end + 2;
                    continue;
                }
            }
        }

        if src[i..].starts_with("~~") {
            if let Some(rel_end) = src[i + 2..].find("~~") {
                if rel_end > 0 {
                    let inner = &src[i + 2..i + 2 + rel_end];
                    let start = u16_pos;
                    let len = inner.encode_utf16().count() as u32;
                    plain.push_str(inner);
                    u16_pos += len;
                    spans.push(MdInlineSpan {
                        start_u16: start,
                        len_u16: len,
                        bold: false,
                        italic: false,
                        strike: true,
                        code: false,
                        math: false,
                        link_url: None,
                    });
                    i = i + 2 + rel_end + 2;
                    continue;
                }
            }
        }

        if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] != b' ' && bytes[i + 1] != b'*' {
            if let Some(rel_end) = src[i + 1..].find('*') {
                if rel_end > 0 && !src[i + 1..i + 1 + rel_end].ends_with(' ') {
                    let inner = &src[i + 1..i + 1 + rel_end];
                    let start = u16_pos;
                    let len = inner.encode_utf16().count() as u32;
                    plain.push_str(inner);
                    u16_pos += len;
                    spans.push(MdInlineSpan {
                        start_u16: start,
                        len_u16: len,
                        bold: false,
                        italic: true,
                        strike: false,
                        code: false,
                        math: false,
                        link_url: None,
                    });
                    i = i + 1 + rel_end + 1;
                    continue;
                }
            }
        }

        let prev_is_alnum = i > 0 && bytes[i - 1].is_ascii_alphanumeric();
        if bytes[i] == b'_'
            && !prev_is_alnum
            && i + 1 < bytes.len()
            && bytes[i + 1] != b' '
            && bytes[i + 1] != b'_'
        {
            if let Some(rel_end) = src[i + 1..].find('_') {
                let after_idx = i + 1 + rel_end + 1;
                let next_is_alnum =
                    after_idx < bytes.len() && bytes[after_idx].is_ascii_alphanumeric();
                if rel_end > 0
                    && !next_is_alnum
                    && !src[i + 1..i + 1 + rel_end].ends_with(' ')
                {
                    let inner = &src[i + 1..i + 1 + rel_end];
                    let start = u16_pos;
                    let len = inner.encode_utf16().count() as u32;
                    plain.push_str(inner);
                    u16_pos += len;
                    spans.push(MdInlineSpan {
                        start_u16: start,
                        len_u16: len,
                        bold: false,
                        italic: true,
                        strike: false,
                        code: false,
                        math: false,
                        link_url: None,
                    });
                    i = after_idx;
                    continue;
                }
            }
        }

        let ch = src[i..].chars().next().unwrap();
        plain.push(ch);
        u16_pos += ch.len_utf16() as u32;
        i += ch.len_utf8();
    }

    MdRichText { plain, spans }
}

fn find_closing_single_dollar(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b'$' {
            return Some(i);
        }
        i += 1;
    }
    None
}

pub fn latex_to_unicode(expr: &str) -> String {
    let s = expr.trim();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];

        if b == b'\\' {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            if !bytes[i].is_ascii_alphabetic() {
                match bytes[i] {
                    b',' | b';' | b':' | b' ' => out.push(' '),
                    b'!' => {}
                    b'\\' => out.push('\n'),
                    b'{' | b'}' | b'$' | b'%' | b'#' | b'_' | b'&' => {
                        out.push(bytes[i] as char);
                    }
                    b'|' => out.push('‖'),
                    other => out.push(other as char),
                }
                i += 1;
                continue;
            }
            let cmd_start = i;
            while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            let cmd = &s[cmd_start..i];
            match cmd {
                "frac" | "dfrac" | "tfrac" => {
                    let (num, next_i) = read_latex_arg(s, i);
                    let (den, next_i2) = read_latex_arg(s, next_i);
                    i = next_i2;
                    let n = latex_to_unicode(&num);
                    let d = latex_to_unicode(&den);
                    match (n.as_str(), d.as_str()) {
                        ("1", "2") => out.push('½'),
                        ("1", "3") => out.push('⅓'),
                        ("2", "3") => out.push('⅔'),
                        ("1", "4") => out.push('¼'),
                        ("3", "4") => out.push('¾'),
                        _ => {
                            let n_needs_p = n.contains(' ')
                                || n.contains('+')
                                || n.contains('-')
                                || n.contains('−');
                            let d_needs_p = d.chars().count() > 1;
                            if n_needs_p {
                                out.push('(');
                                out.push_str(&n);
                                out.push(')');
                            } else {
                                out.push_str(&n);
                            }
                            out.push('/');
                            if d_needs_p {
                                out.push('(');
                                out.push_str(&d);
                                out.push(')');
                            } else {
                                out.push_str(&d);
                            }
                        }
                    }
                }
                "sqrt" => {
                    let mut idx_str = String::new();
                    let mut p = skip_ws(s, i);
                    if p < bytes.len() && bytes[p] == b'[' {
                        if let Some(rel) = s[p + 1..].find(']') {
                            idx_str = latex_to_unicode(&s[p + 1..p + 1 + rel]);
                            p = p + 1 + rel + 1;
                        }
                    }
                    let (arg, next_i) = read_latex_arg(s, p);
                    i = next_i;
                    let inner = latex_to_unicode(&arg);
                    if idx_str == "3" {
                        out.push('∛');
                    } else if idx_str == "4" {
                        out.push('∜');
                    } else {
                        if !idx_str.is_empty() {
                            out.push_str(&to_superscript(&idx_str).unwrap_or(idx_str));
                        }
                        out.push('√');
                    }
                    if inner.chars().count() > 1 {
                        out.push('(');
                        out.push_str(&inner);
                        out.push(')');
                    } else {
                        out.push_str(&inner);
                    }
                }
                "mathbb" => {
                    let (arg, next_i) = read_latex_arg(s, i);
                    i = next_i;
                    for ch in arg.trim().chars() {
                        out.push(match ch {
                            'R' => 'ℝ',
                            'N' => 'ℕ',
                            'Z' => 'ℤ',
                            'Q' => 'ℚ',
                            'C' => 'ℂ',
                            'P' => 'ℙ',
                            'E' => '𝔼',
                            other => other,
                        });
                    }
                }
                "text" | "mathrm" | "mathbf" | "mathit" | "mathsf" | "mathtt" | "operatorname"
                | "boldsymbol" | "mathcal" | "mathfrak" => {
                    let (arg, next_i) = read_latex_arg(s, i);
                    i = next_i;
                    out.push_str(&latex_to_unicode(&arg));
                }
                "hat" | "widehat" => {
                    let (arg, next_i) = read_latex_arg(s, i);
                    i = next_i;
                    out.push_str(&latex_to_unicode(&arg));
                    out.push('\u{0302}');
                }
                "bar" | "overline" => {
                    let (arg, next_i) = read_latex_arg(s, i);
                    i = next_i;
                    out.push_str(&latex_to_unicode(&arg));
                    out.push('\u{0304}');
                }
                "vec" | "overrightarrow" => {
                    let (arg, next_i) = read_latex_arg(s, i);
                    i = next_i;
                    out.push_str(&latex_to_unicode(&arg));
                    out.push('\u{20D7}');
                }
                "dot" => {
                    let (arg, next_i) = read_latex_arg(s, i);
                    i = next_i;
                    out.push_str(&latex_to_unicode(&arg));
                    out.push('\u{0307}');
                }
                "tilde" | "widetilde" => {
                    let (arg, next_i) = read_latex_arg(s, i);
                    i = next_i;
                    out.push_str(&latex_to_unicode(&arg));
                    out.push('\u{0303}');
                }
                "left" | "right" | "big" | "Big" | "bigg" | "Bigg" | "displaystyle"
                | "textstyle" | "limits" | "nolimits" => {
                    let p = skip_ws(s, i);
                    if p < bytes.len() && bytes[p] == b'.' {
                        i = p + 1;
                    }
                }
                "quad" | "qquad" => out.push_str("  "),
                "alpha" => out.push('α'),
                "beta" => out.push('β'),
                "gamma" => out.push('γ'),
                "delta" => out.push('δ'),
                "epsilon" | "varepsilon" => out.push('ε'),
                "zeta" => out.push('ζ'),
                "eta" => out.push('η'),
                "theta" | "vartheta" => out.push('θ'),
                "iota" => out.push('ι'),
                "kappa" => out.push('κ'),
                "lambda" => out.push('λ'),
                "mu" => out.push('μ'),
                "nu" => out.push('ν'),
                "xi" => out.push('ξ'),
                "pi" | "varpi" => out.push('π'),
                "rho" | "varrho" => out.push('ρ'),
                "sigma" | "varsigma" => out.push('σ'),
                "tau" => out.push('τ'),
                "upsilon" => out.push('υ'),
                "phi" | "varphi" => out.push('φ'),
                "chi" => out.push('χ'),
                "psi" => out.push('ψ'),
                "omega" => out.push('ω'),
                "Gamma" => out.push('Γ'),
                "Delta" => out.push('Δ'),
                "Theta" => out.push('Θ'),
                "Lambda" => out.push('Λ'),
                "Xi" => out.push('Ξ'),
                "Pi" => out.push('Π'),
                "Sigma" => out.push('Σ'),
                "Upsilon" => out.push('Υ'),
                "Phi" => out.push('Φ'),
                "Psi" => out.push('Ψ'),
                "Omega" => out.push('Ω'),
                "times" => out.push('×'),
                "cdot" | "centerdot" => out.push('·'),
                "div" => out.push('÷'),
                "pm" => out.push('±'),
                "mp" => out.push('∓'),
                "neq" | "ne" => out.push('≠'),
                "leq" | "le" => out.push('≤'),
                "geq" | "ge" => out.push('≥'),
                "ll" => out.push('≪'),
                "gg" => out.push('≫'),
                "approx" => out.push('≈'),
                "equiv" => out.push('≡'),
                "sim" => out.push('∼'),
                "simeq" => out.push('≃'),
                "cong" => out.push('≅'),
                "propto" => out.push('∝'),
                "infty" => out.push('∞'),
                "partial" => out.push('∂'),
                "nabla" => out.push('∇'),
                "sum" => out.push('∑'),
                "prod" => out.push('∏'),
                "int" => out.push('∫'),
                "iint" => out.push('∬'),
                "iiint" => out.push('∭'),
                "oint" => out.push('∮'),
                "forall" => out.push('∀'),
                "exists" => out.push('∃'),
                "nexists" => out.push('∄'),
                "in" => out.push('∈'),
                "notin" => out.push('∉'),
                "ni" => out.push('∋'),
                "subset" => out.push('⊂'),
                "subseteq" => out.push('⊆'),
                "supset" => out.push('⊃'),
                "supseteq" => out.push('⊇'),
                "cup" => out.push('∪'),
                "cap" => out.push('∩'),
                "emptyset" | "varnothing" => out.push('∅'),
                "setminus" => out.push('∖'),
                "land" | "wedge" => out.push('∧'),
                "lor" | "vee" => out.push('∨'),
                "neg" | "lnot" => out.push('¬'),
                "to" | "rightarrow" => out.push('→'),
                "leftarrow" | "gets" => out.push('←'),
                "leftrightarrow" => out.push('↔'),
                "Rightarrow" | "implies" => out.push('⇒'),
                "Leftarrow" => out.push('⇐'),
                "Leftrightarrow" | "iff" => out.push('⇔'),
                "mapsto" => out.push('↦'),
                "dots" | "ldots" => out.push('…'),
                "cdots" => out.push('⋯'),
                "vdots" => out.push('⋮'),
                "ddots" => out.push('⋱'),
                "circ" => out.push('∘'),
                "deg" => out.push('°'),
                "angle" => out.push('∠'),
                "perp" => out.push('⊥'),
                "parallel" => out.push('∥'),
                "therefore" => out.push('∴'),
                "because" => out.push('∵'),
                "star" => out.push('⋆'),
                "oplus" => out.push('⊕'),
                "otimes" => out.push('⊗'),
                "langle" => out.push('⟨'),
                "rangle" => out.push('⟩'),
                "lfloor" => out.push('⌊'),
                "rfloor" => out.push('⌋'),
                "lceil" => out.push('⌈'),
                "rceil" => out.push('⌉'),
                "hbar" => out.push('ℏ'),
                "ell" => out.push('ℓ'),
                "Re" => out.push('ℜ'),
                "Im" => out.push('ℑ'),
                "aleph" => out.push('ℵ'),
                "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "arcsin" | "arccos" | "arctan"
                | "sinh" | "cosh" | "tanh" | "log" | "ln" | "exp" | "lim" | "max" | "min"
                | "sup" | "inf" | "det" | "dim" | "ker" | "gcd" | "lcm" | "mod" | "arg" => {
                    out.push_str(cmd);
                }
                other => {
                    out.push_str(other);
                }
            }
            continue;
        }

        if b == b'^' {
            let (arg, next_i) = read_latex_arg(s, i + 1);
            i = next_i;
            let inner = latex_to_unicode(&arg);
            if inner == "∘" || inner == "°" {
                out.push('°');
            } else if let Some(sup) = to_superscript(&inner) {
                out.push_str(&sup);
            } else if inner.chars().count() > 1 {
                out.push_str("^(");
                out.push_str(&inner);
                out.push(')');
            } else {
                out.push('^');
                out.push_str(&inner);
            }
            continue;
        }

        if b == b'_' {
            let (arg, next_i) = read_latex_arg(s, i + 1);
            i = next_i;
            let inner = latex_to_unicode(&arg);
            if let Some(sub) = to_subscript(&inner) {
                out.push_str(&sub);
            } else if inner.chars().count() > 1 {
                out.push_str("_(");
                out.push_str(&inner);
                out.push(')');
            } else {
                out.push('_');
                out.push_str(&inner);
            }
            continue;
        }

        if b == b'{' || b == b'}' {
            i += 1;
            continue;
        }

        let ch = s[i..].chars().next().unwrap();
        if ch == '-' {
            out.push('−');
        } else {
            out.push(ch);
        }
        i += ch.len_utf8();
    }

    let mut cleaned = String::with_capacity(out.len());
    let mut prev_space = false;
    for ch in out.chars() {
        if ch == ' ' {
            if !prev_space {
                cleaned.push(' ');
            }
            prev_space = true;
        } else {
            cleaned.push(ch);
            prev_space = false;
        }
    }
    cleaned.trim().to_string()
}

fn skip_ws(s: &str, mut i: usize) -> usize {
    let bytes = s.as_bytes();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn read_latex_arg(s: &str, start: usize) -> (String, usize) {
    let i = skip_ws(s, start);
    let bytes = s.as_bytes();
    if i >= bytes.len() {
        return (String::new(), i);
    }
    if bytes[i] == b'{' {
        let mut depth = 1usize;
        let mut p = i + 1;
        while p < bytes.len() && depth > 0 {
            if bytes[p] == b'\\' && p + 1 < bytes.len() {
                p += 2;
                continue;
            }
            if bytes[p] == b'{' {
                depth += 1;
            } else if bytes[p] == b'}' {
                depth -= 1;
                if depth == 0 {
                    return (s[i + 1..p].to_string(), p + 1);
                }
            }
            p += 1;
        }
        return (s[i + 1..].to_string(), bytes.len());
    }
    if bytes[i] == b'\\' {
        let mut p = i + 1;
        while p < bytes.len() && bytes[p].is_ascii_alphabetic() {
            p += 1;
        }
        if p == i + 1 && p < bytes.len() {
            p += 1;
        }
        return (s[i..p].to_string(), p);
    }
    let ch = s[i..].chars().next().unwrap();
    (ch.to_string(), i + ch.len_utf8())
}

fn to_superscript(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let mapped = match ch {
            '0' => '⁰',
            '1' => '¹',
            '2' => '²',
            '3' => '³',
            '4' => '⁴',
            '5' => '⁵',
            '6' => '⁶',
            '7' => '⁷',
            '8' => '⁸',
            '9' => '⁹',
            '+' => '⁺',
            '-' | '−' => '⁻',
            '=' => '⁼',
            '(' => '⁽',
            ')' => '⁾',
            'n' => 'ⁿ',
            'i' => 'ⁱ',
            'x' => 'ˣ',
            'y' => 'ʸ',
            'a' => 'ᵃ',
            'b' => 'ᵇ',
            'c' => 'ᶜ',
            'd' => 'ᵈ',
            'e' => 'ᵉ',
            'f' => 'ᶠ',
            'g' => 'ᵍ',
            'h' => 'ʰ',
            'j' => 'ʲ',
            'k' => 'ᵏ',
            'l' => 'ˡ',
            'm' => 'ᵐ',
            'o' => 'ᵒ',
            'p' => 'ᵖ',
            'r' => 'ʳ',
            's' => 'ˢ',
            't' => 'ᵗ',
            'u' => 'ᵘ',
            'v' => 'ᵛ',
            'w' => 'ʷ',
            'z' => 'ᶻ',
            ' ' => ' ',
            _ => return None,
        };
        out.push(mapped);
    }
    Some(out)
}

fn to_subscript(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let mapped = match ch {
            '0' => '₀',
            '1' => '₁',
            '2' => '₂',
            '3' => '₃',
            '4' => '₄',
            '5' => '₅',
            '6' => '₆',
            '7' => '₇',
            '8' => '₈',
            '9' => '₉',
            '+' => '₊',
            '-' | '−' => '₋',
            '=' => '₌',
            '(' => '₍',
            ')' => '₎',
            'a' => 'ₐ',
            'e' => 'ₑ',
            'h' => 'ₕ',
            'i' => 'ᵢ',
            'j' => 'ⱼ',
            'k' => 'ₖ',
            'l' => 'ₗ',
            'm' => 'ₘ',
            'n' => 'ₙ',
            'o' => 'ₒ',
            'p' => 'ₚ',
            'r' => 'ᵣ',
            's' => 'ₛ',
            't' => 'ₜ',
            'u' => 'ᵤ',
            'v' => 'ᵥ',
            'x' => 'ₓ',
            ' ' => ' ',
            _ => return None,
        };
        out.push(mapped);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_single_and_double_newlines() {
        let blocks = parse_markdown("Prima riga\nSeconda riga\n\nSecondo paragrafo");
        assert_eq!(blocks.len(), 2);
        match &blocks[0] {
            MdBlock::Paragraph(rt) => assert_eq!(rt.plain, "Prima riga\nSeconda riga"),
            other => panic!("expected Paragraph, got {other:?}"),
        }
        match &blocks[1] {
            MdBlock::Paragraph(rt) => assert_eq!(rt.plain, "Secondo paragrafo"),
            other => panic!("expected Paragraph, got {other:?}"),
        }
    }

    #[test]
    fn parses_inline_styles_and_keeps_snake_case() {
        let rt = parse_inline(
            "**grassetto** *corsivo* `my_var` ~~barra~~ [link](https://x.com) e https://rust-lang.org.",
        );
        assert_eq!(
            rt.plain,
            "grassetto corsivo my_var barra link e https://rust-lang.org."
        );
        assert!(rt.spans.iter().any(|s| s.bold));
        assert!(rt.spans.iter().any(|s| s.italic));
        assert!(rt.spans.iter().any(|s| s.code));
        assert!(rt.spans.iter().any(|s| s.strike));
        assert!(rt
            .spans
            .iter()
            .any(|s| s.link_url.as_deref() == Some("https://x.com")));
        assert!(rt
            .spans
            .iter()
            .any(|s| s.link_url.as_deref() == Some("https://rust-lang.org")));
    }

    #[test]
    fn parses_headings_lists_code_quote_hr_and_table() {
        let md = "\
# Titolo
- Uno
- Due
1. Primo
> Citazione
---
```rust
fn main() {}
```
| A | B |
|---|---|
| 1 | 2 |";
        let blocks = parse_markdown(md);
        assert!(matches!(blocks[0], MdBlock::Heading { level: 1, .. }));
        assert!(matches!(blocks[1], MdBlock::BulletItem { .. }));
        assert!(matches!(blocks[2], MdBlock::BulletItem { .. }));
        assert!(matches!(blocks[3], MdBlock::NumberedItem { .. }));
        assert!(matches!(blocks[4], MdBlock::Blockquote(_)));
        assert!(matches!(blocks[5], MdBlock::HorizontalRule));
        assert!(matches!(blocks[6], MdBlock::CodeBlock { .. }));
        assert!(matches!(blocks[7], MdBlock::Table { .. }));
    }

    #[test]
    fn handles_unclosed_streaming_code_block() {
        let blocks = parse_markdown("Ecco:\n```python\nprint('hi')");
        assert_eq!(blocks.len(), 2);
        match &blocks[1] {
            MdBlock::CodeBlock { lang, code } => {
                assert_eq!(lang, "python");
                assert_eq!(code, "print('hi')");
            }
            other => panic!("expected CodeBlock, got {other:?}"),
        }
    }

    #[test]
    fn renders_inline_and_display_latex_math() {
        assert_eq!(latex_to_unicode("E = mc^2"), "E = mc²");
        assert_eq!(
            latex_to_unicode(r"\frac{-b \pm \sqrt{b^2 - 4ac}}{2a}"),
            "(−b ± √(b² − 4ac))/(2a)"
        );
        assert_eq!(latex_to_unicode(r"\sum_{i=1}^{n} x_i \in \mathbb{R}"), "∑ᵢ₌₁ⁿ xᵢ ∈ ℝ");
        assert_eq!(latex_to_unicode(r"\alpha + \beta \leq \infty"), "α + β ≤ ∞");

        let rt = parse_inline(r"Costa $100 ma $E = mc^2$ e \(\pi r^2\) funzionano.");
        assert_eq!(rt.plain, "Costa $100 ma E = mc² e π r² funzionano.");
        assert_eq!(rt.spans.iter().filter(|s| s.math).count(), 2);

        let blocks = parse_markdown("Formula:\n$$\n\\int_0^\\infty e^{-x} dx = 1\n$$");
        assert_eq!(blocks.len(), 2);
        match &blocks[1] {
            MdBlock::MathBlock(m) => assert!(m.contains('∫') && m.contains('∞')),
            other => panic!("expected MathBlock, got {other:?}"),
        }
    }
}

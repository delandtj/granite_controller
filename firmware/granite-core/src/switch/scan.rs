//! The hand-written HTML scanner the page parsers work from (ADR 0006
//! component 15: "a small hand-written scanner ...; no `regex` crate").
//!
//! It is not an HTML parser and does not try to be one. The Sodola CGI
//! pages are machine-generated from one template per page, so four
//! primitives cover every page the controller reads:
//!
//! - [`rows`]: the `<td>` cells of every `<tr>` of a fragment, tags
//!   stripped and the handful of entities the firmware emits unescaped,
//! - [`attr`]: the `value` of the tag that carries `name="<field>"`,
//!   which is how every text input is read (`ip.cgi` writes its inputs
//!   from JavaScript with single quotes, so both quote styles count),
//! - [`selected`]: the `value` of the `<option ... selected>` of one
//!   `<select name="...">`,
//! - [`checked`]: whether the input called `<field>` carries `checked`.
//!
//! Everything here works on `&str` and allocates only what it returns,
//! so a page can be parsed in place and dropped (component 16: "no page
//! outlives its parse").

use alloc::string::String;
use alloc::vec::Vec;

/// The part of `html` after the first occurrence of `marker`, or the
/// whole of it when the marker is not there.
///
/// Several pages carry the same table twice - the form that writes it
/// and the table that shows it - and the marker is what tells them
/// apart (`class="showdiv"`, `getRmvVlanEntry`, ...).
pub fn after<'a>(html: &'a str, marker: &str) -> &'a str {
    match html.find(marker) {
        Some(at) => &html[at + marker.len()..],
        None => html,
    }
}

/// The part of `html` before the first occurrence of `marker`, or the
/// whole of it when the marker is not there.
pub fn before<'a>(html: &'a str, marker: &str) -> &'a str {
    match html.find(marker) {
        Some(at) => &html[..at],
        None => html,
    }
}

/// True when `html` is the login redirect the CGI answers with once the
/// switch has forgotten the session (`docs/sodola.md`, "Protocol").
///
/// The body is a three-line page whose script replaces the location with
/// `/login.cgi`; nothing else on the switch does that.
pub fn is_login_redirect(html: &str) -> bool {
    html.contains("login.cgi") && html.contains("location.replace")
}

/// Every `<tr>` of `html` as its list of `<td>` cells.
///
/// Rows with no `<td>` (the `<th>` header rows) are left out, so a
/// caller can match on the cell count of a data row.
pub fn rows(html: &str) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut rest = html;
    while let Some(start) = find_tag(rest, "tr") {
        rest = &rest[start..];
        let body_start = match rest.find('>') {
            Some(at) => at + 1,
            None => break,
        };
        let (body, next) = match rest[body_start..].find("</tr") {
            Some(end) => (
                &rest[body_start..body_start + end],
                &rest[body_start + end..],
            ),
            None => (&rest[body_start..], ""),
        };
        let cells = cells(body);
        if !cells.is_empty() {
            out.push(cells);
        }
        rest = next;
        if rest.is_empty() {
            break;
        }
    }
    out
}

/// The `<td>` cells of one row body, tags stripped and trimmed.
pub fn cells(row: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = row;
    while let Some(start) = find_tag(rest, "td") {
        rest = &rest[start..];
        let body_start = match rest.find('>') {
            Some(at) => at + 1,
            None => break,
        };
        let (body, next) = match rest[body_start..].find("</td") {
            Some(end) => (
                &rest[body_start..body_start + end],
                &rest[body_start + end..],
            ),
            None => (&rest[body_start..], ""),
        };
        out.push(text(body));
        rest = next;
        if rest.is_empty() {
            break;
        }
    }
    out
}

/// `html` with every tag dropped, entities unescaped and the whitespace
/// collapsed to single spaces.
pub fn text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if in_tag => {
                let _ = c;
            }
            c if c.is_ascii_whitespace() => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            c => out.push(c),
        }
    }
    unescape(out.trim())
}

/// The five entities the switch's pages use.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return String::from(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let mut matched = false;
        for (entity, ch) in [
            ("&amp;", '&'),
            ("&lt;", '<'),
            ("&gt;", '>'),
            ("&quot;", '"'),
            ("&#39;", '\''),
            ("&nbsp;", ' '),
        ] {
            if rest.starts_with(entity) {
                out.push(ch);
                rest = &rest[entity.len()..];
                matched = true;
                break;
            }
        }
        if !matched {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Start of the next `<tag ...>` or `<tag>` in `html`, skipping names
/// that only begin with `tag` (`<table>` is not a `<td>`).
fn find_tag(html: &str, tag: &str) -> Option<usize> {
    let mut from = 0usize;
    while let Some(at) = html[from..].find('<') {
        let at = from + at;
        let rest = &html[at + 1..];
        if let Some(tail) = strip_ci(rest, tag)
            && tail
                .chars()
                .next()
                .is_some_and(|c| c == '>' || c.is_ascii_whitespace())
        {
            return Some(at);
        }
        from = at + 1;
        if from >= html.len() {
            break;
        }
    }
    None
}

/// `s` without a case-insensitive `prefix`.
fn strip_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let (head, tail) = s.split_at_checked(prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then_some(tail)
}

/// The tag that carries `name="<field>"`, as the slice from the opening
/// `<` to the closing `>`.
///
/// Both quote styles and the bare form count: the port checkboxes are
/// written as `name=port0` and `ip.cgi` builds its inputs in JavaScript
/// with single quotes.
pub fn tag_named<'a>(html: &'a str, field: &str) -> Option<&'a str> {
    let mut from = 0usize;
    loop {
        let at = from + html[from..].find("name=")?;
        let after_eq = &html[at + 5..];
        let (value, rest) = quoted(after_eq)?;
        if value == field {
            let start = html[..at].rfind('<')?;
            let used = html.len() - rest.len();
            let end = html[used..].find('>').map_or(html.len(), |e| used + e + 1);
            return Some(&html[start..end]);
        }
        from = at + 5;
        if from >= html.len() {
            return None;
        }
    }
}

/// The attribute value at the start of `s`, with the rest after it.
/// Handles `"x"`, `'x'` and a bare `x` ended by whitespace or `>`.
fn quoted(s: &str) -> Option<(&str, &str)> {
    let mut chars = s.char_indices();
    let (_, first) = chars.next()?;
    match first {
        '"' | '\'' => {
            let end = s[1..].find(first)? + 1;
            Some((&s[1..end], &s[end + 1..]))
        }
        _ => {
            let end = s
                .find(|c: char| c == '>' || c.is_ascii_whitespace())
                .unwrap_or(s.len());
            Some((&s[..end], &s[end..]))
        }
    }
}

/// The `value` of the input called `field`.
pub fn attr(html: &str, field: &str) -> Option<String> {
    let tag = tag_named(html, field)?;
    let at = tag.find("value=")?;
    let (value, _) = quoted(&tag[at + 6..])?;
    Some(unescape(value))
}

/// True when the input called `field` is there and carries `checked`.
pub fn checked(html: &str, field: &str) -> bool {
    tag_named(html, field).is_some_and(|tag| {
        tag.split([' ', '\t', '\n', '\r', '>'])
            .any(|word| word.eq_ignore_ascii_case("checked"))
    })
}

/// True when an input called `field` is on the page at all.
pub fn has_field(html: &str, field: &str) -> bool {
    tag_named(html, field).is_some()
}

/// One `<option>` of a `<select>`: the `value` attribute and the label
/// after the tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Option_ {
    /// The `value` attribute.
    pub value: String,
    /// The text after the tag, up to the end of the line.
    pub label: String,
    /// Whether the option carries `selected`.
    pub selected: bool,
}

/// Every `<option>` of the `<select name="<field>">`.
///
/// The firmware leaves its `<option>` tags unclosed, so an option ends
/// at the next `<`.
pub fn options(html: &str, field: &str) -> Vec<Option_> {
    let mut out: Vec<Option_> = Vec::new();
    let Some(tag) = tag_named(html, field) else {
        return out;
    };
    let select_at = match html.find(tag) {
        Some(at) => at + tag.len(),
        None => return out,
    };
    let body = before(&html[select_at..], "</select");
    let mut rest = body;
    while let Some(start) = find_tag(rest, "option") {
        rest = &rest[start..];
        let Some(close) = rest.find('>') else { break };
        let open = &rest[..close];
        let label_end = rest[close + 1..]
            .find('<')
            .map_or(rest.len(), |at| close + 1 + at);
        let label = text(&rest[close + 1..label_end]);
        let value = open
            .find("value=")
            .and_then(|at| quoted(&open[at + 6..]).map(|(v, _)| String::from(v)))
            .unwrap_or_default();
        out.push(Option_ {
            value,
            label,
            selected: open
                .split([' ', '\t', '\n', '\r'])
                .any(|word| word.eq_ignore_ascii_case("selected")),
        });
        rest = &rest[label_end..];
        if rest.is_empty() {
            break;
        }
    }
    out
}

/// The selected option of the `<select name="<field>">`.
pub fn selected(html: &str, field: &str) -> Option<Option_> {
    options(html, field).into_iter().find(|o| o.selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"
<table><tr><th>Port</th><th>State</th></tr>
<tr><td>Port 1</td><td>Enable</td>		<td>Auto</td></tr>
<tr><td>Port 2</td><td>Disable&amp;</td><td>  Half
Duplex </td></tr></table>
<input name="enable_jumbo" type="checkbox" checked>
<input type=checkbox id=port0 name=port0 class="chkp">
<select name="jumboframe">
<option value="1">1522Bytes
<option value="6" selected>12000Bytes
</select>
<input type='text' name='ip' id='ip' value='192.168.64.241' maxlength='15'>
"#;

    #[test]
    fn rows_are_the_td_cells_of_every_tr() {
        let r = rows(PAGE);
        assert_eq!(r.len(), 2, "the th header row has no cells");
        assert_eq!(r[0], ["Port 1", "Enable", "Auto"]);
        assert_eq!(
            r[1],
            ["Port 2", "Disable&", "Half Duplex"],
            "entities unescaped, whitespace collapsed"
        );
    }

    #[test]
    fn inputs_selects_and_checkboxes_read_back() {
        assert!(checked(PAGE, "enable_jumbo"));
        assert!(!checked(PAGE, "port0"), "an unchecked box is not checked");
        assert!(!checked(PAGE, "nothing_called_this"));
        assert!(has_field(PAGE, "port0"));
        assert!(!has_field(PAGE, "port3"));
        assert_eq!(attr(PAGE, "ip").as_deref(), Some("192.168.64.241"));
        let sel = selected(PAGE, "jumboframe").expect("an option is selected");
        assert_eq!(sel.value, "6");
        assert_eq!(sel.label, "12000Bytes");
        assert_eq!(options(PAGE, "jumboframe").len(), 2);
    }

    #[test]
    fn a_login_redirect_is_recognised() {
        assert!(is_login_redirect(
            "<script>window.top.location.replace(\"/login.cgi\");</script>"
        ));
        assert!(!is_login_redirect(PAGE));
    }
}

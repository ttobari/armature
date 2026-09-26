//! md を1枚の HTML に落とす小さな組版。**アプリ内ブラウザに渡すためだけ**。
//!
//! html や画像は WebView が素で描けるので、ここに来るのは md だけ。md を素で渡すと
//! **タグの無いただの字**が1行に繋がって出る。
//!
//! 扱うのは
//! 冒頭のメタ(YAML front matter)/ 見出し / 箇条書き(入れ子)/ 番号付き /
//! 済み札(`- [ ]`)/ 引用 / 囲みコード / 表 / 区切り / 段落、
//! 行の中は太字・斜体・打ち消し・コード・リンク。
//!
//! 脚注も画像も持たない。**持たせない**——凝った記法を追い始めると、markdown 実装の
//! 再実装になる。生の HTML も通さない(escape する)——例外は畳む節の
//! `<details>` / `<summary>` / `<br>` だけで、属性が付いていたら字として見せる。

use std::fmt::Write as _;

/// md を1枚の HTML にする。`title` は窓の題(ファイル名を渡す)。
#[must_use]
pub fn to_html(source: &str, title: &str) -> String {
    let (meta, rest) = match front_matter(source) {
        Some((yaml, rest)) => (front_matter_html(yaml), rest),
        None => (String::new(), source),
    };
    format!(
        "<!DOCTYPE html><html lang=\"ja\"><head><meta charset=\"utf-8\">\n\
         <title>{}</title>\n<style>{}</style></head>\n<body><main>\n{}{}</main></body></html>\n",
        escape(title),
        style(),
        meta,
        body(rest)
    )
}

/// 冒頭の YAML メタと、それを除いた本文。`---` の行で囲まれた塊だけを見る。
///
/// task md は全部これを持つので、素通しすると**区切り線+潰れた1段落**になって
/// 読めない(`ID: 861 親: 状態: 進行 …` と1行に繋がる)。閉じの `---` が
/// 無ければメタとみなさない——本文の区切り線を食わないため。
fn front_matter(source: &str) -> Option<(&str, &str)> {
    let after = source.strip_prefix("---\n")?;
    let (end, _) = after
        .match_indices("\n---")
        .find(|(at, _)| matches!(after[at + 4..].chars().next(), None | Some('\n')))?;
    Some((&after[..end], after[end + 4..].trim_start_matches('\n')))
}

/// メタを見出しの前の小さな札にする。`key: value` だけを見て、割れない行は
/// そのまま1行見せる(配列やネストが来ても落とさない)。
fn front_matter_html(yaml: &str) -> String {
    let mut out = String::from("<dl class=\"meta\">");
    for line in yaml.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        match line.split_once(':') {
            Some((key, value)) if !key.trim().is_empty() && !key.starts_with(' ') => {
                let value = value.trim();
                let _ = write!(out, "<dt>{}</dt>", inline(key.trim()));
                if value.is_empty() {
                    // 空欄は消さずに見せる——「無い」も読み取れる情報。
                    out.push_str("<dd class=\"empty\">—</dd>");
                } else {
                    let _ = write!(out, "<dd>{}</dd>", inline(value));
                }
            }
            _ => {
                let _ = write!(out, "<dd class=\"raw\">{}</dd>", inline(line.trim()));
            }
        }
    }
    out.push_str("</dl>\n");
    out
}

/// 素の字を1枚の HTML にする。**組版はしない**——`<pre>` に流すだけ。
///
/// ファイルツリーから開くのは `.rs` や `.toml` のような
/// 素のコードで、[`to_html`] に通すと `#` が見出しに `*` が強調に化け、字下げも
/// 潰れる。**同じ「HTMLにして渡す」でも、解釈するかしないかが違う。**
#[must_use]
pub fn code_to_html(source: &str, title: &str) -> String {
    format!(
        "<!DOCTYPE html><html lang=\"ja\"><head><meta charset=\"utf-8\">\n\
         <title>{}</title>\n<style>{}{}</style></head>\n\
         <body><main class=\"plain\"><pre><code>{}</code></pre></main></body></html>\n",
        escape(title),
        style(),
        code_style(),
        escape(source)
    )
}

/// 素の字の面だけの足し。土台は [`style`] と同じ(同じ窓の中で色を割らない)。
///
/// 読む面いっぱいに広げるのは、コードが 860px で折り返されると桁が揃わないから
/// ——`pre` の囲みも消す(頁ぜんぶがコードなので、囲む相手が居ない)。
fn code_style() -> String {
    "main.plain{max-width:none}\
     main.plain pre{background:none;border-radius:0;padding:0;\
     font-size:13px;line-height:1.6}\
     main.plain code{font-family:'SFMono-Regular',Menlo,monospace;white-space:pre}"
        .to_string()
}

/// 地と字と面は窓の絵の具(`palette`)から取る。**新しい色は作らない**
/// ——同じ窓の中で、ページだけ別の世界の色になるのを避ける。
/// 見出しは金(`palette::yellow`)。
fn style() -> String {
    let css = |c: iced::Color| {
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}", byte(c.r), byte(c.g), byte(c.b))
    };
    let window = crate::palette::surface_window();
    let scheme = if window.r * 0.299 + window.g * 0.587 + window.b * 0.114 < 0.588 {
        "dark"
    } else {
        "light"
    };
    format!(
        ":root{{color-scheme:{scheme}}}\
         html{{background:{base}}}\
         body{{background:{base};color:{text};margin:0;padding:28px 32px;\
         font-family:-apple-system,'Hiragino Sans',sans-serif;line-height:1.9}}\
         main{{max-width:860px;margin:0 auto}}\
         h1,h2,h3,h4,h5,h6{{color:{gold};line-height:1.5;margin:1.6em 0 .6em}}\
         h1{{font-size:24px;border-bottom:1px solid {surf2};padding-bottom:.3em}}\
         h2{{font-size:20px}}h3{{font-size:17px}}h4,h5,h6{{font-size:15px}}\
         p{{margin:.8em 0}}ul,ol{{margin:.6em 0;padding-left:1.6em}}li{{margin:.2em 0}}\
         ul ul,ul ol,ol ul,ol ol{{margin:0}}\
         blockquote{{margin:.8em 0;padding:.1em 1em;border-left:3px solid {surf2};color:{sub}}}\
         hr{{border:0;border-top:1px solid {surf2};margin:1.6em 0}}\
         code{{background:{surf1};border-radius:4px;padding:.1em .35em;font-size:.92em;\
         font-family:'SFMono-Regular',Menlo,monospace}}\
         pre{{background:{surf1};border-radius:8px;padding:12px 14px;overflow-x:auto}}\
         pre code{{background:none;padding:0}}\
         strong code{{font-weight:700}}\
         a{{color:{gold}}}em{{color:{sub}}}\
         del{{color:{sub}}}\
         details{{margin:.8em 0}}summary{{color:{gold};cursor:pointer}}\
         dl.meta{{display:grid;grid-template-columns:max-content 1fr;gap:.1em 1.2em;\
         margin:0 0 1.8em;padding:.8em 1.1em;border:1px solid {surf2};border-radius:8px;\
         font-size:13px;line-height:1.8}}\
         dl.meta dt{{color:{gold}}}dl.meta dd{{margin:0}}\
         dl.meta dd.empty{{color:{sub}}}dl.meta dd.raw{{grid-column:1/-1;color:{sub}}}\
         li.task{{list-style:none;margin-left:-1.3em}}\
         li.task input{{margin-right:.4em;accent-color:{gold}}}\
         table{{border-collapse:collapse;margin:1em 0;display:block;overflow-x:auto}}\
         th,td{{border:1px solid {surf2};padding:.4em .9em;line-height:1.7}}\
         th{{background:{surf1};color:{gold};font-weight:600;white-space:nowrap}}",
        base = css(window),
        text = css(crate::palette::text_primary()),
        gold = css(crate::palette::yellow()),
        surf1 = css(crate::palette::surface_raised()),
        surf2 = css(crate::palette::surface_active()),
        sub = css(crate::palette::text_muted()),
    )
}

/// 本文。行を上から読んで塊ごとに閉じる。
fn body(source: &str) -> String {
    let mut out = String::new();
    let mut para: Vec<String> = Vec::new();
    let mut lists = Lists::default();
    let mut quote = false;
    let mut fence: Option<(usize, String)> = None;

    // 表は仕切り行を見てから決まるので、1行だけ先を覗けるようにする。
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.next() {
        // 囲みコードの中は1字も解釈しない。**閉じるのは開いた数以上の印だけ**
        // ——4本で開けた囲みの中に3本の囲みを書く形(md の入れ子)を壊さない。
        if let Some((ticks, code)) = fence.as_mut() {
            if backticks(line).is_some_and(|got| got >= *ticks) {
                let _ = writeln!(out, "<pre><code>{}</code></pre>", escape(code));
                fence = None;
            } else {
                code.push_str(line);
                code.push('\n');
            }
            continue;
        }
        if let Some(ticks) = backticks(line) {
            flush(&mut out, &mut para, &mut lists, &mut quote);
            fence = Some((ticks, String::new()));
            continue;
        }

        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            flush(&mut out, &mut para, &mut lists, &mut quote);
            continue;
        }
        // 区切り。
        if is_rule(trimmed.trim()) {
            flush(&mut out, &mut para, &mut lists, &mut quote);
            out.push_str("<hr>\n");
            continue;
        }
        // 見出し。
        if let Some((level, text)) = heading(trimmed.trim_start()) {
            flush(&mut out, &mut para, &mut lists, &mut quote);
            let _ = writeln!(out, "<h{level}>{}</h{level}>", inline(text));
            continue;
        }
        // 表。仕切り行が続いていて初めて表とみなす——パイプを含むだけの
        // 段落(コマンド例など)を表に化けさせない。
        if is_table_row(trimmed) && lines.peek().is_some_and(|next| is_table_divider(next)) {
            flush(&mut out, &mut para, &mut lists, &mut quote);
            let divider = lines.next().unwrap_or_default();
            let mut rows: Vec<&str> = Vec::new();
            while lines
                .peek()
                .is_some_and(|next| is_table_row(next.trim_end()))
            {
                if let Some(row) = lines.next() {
                    rows.push(row.trim_end());
                }
            }
            out.push_str(&table(trimmed, divider, &rows));
            continue;
        }
        // 畳む節。生の HTML はここだけ通す(属性なしの完全一致に限る)。
        if let Some(html) = html_block(trimmed.trim()) {
            flush(&mut out, &mut para, &mut lists, &mut quote);
            out.push_str(&html);
            continue;
        }
        // 引用。
        if let Some(rest) = trimmed.trim_start().strip_prefix('>') {
            lists.close(&mut out);
            if !quote {
                flush_para(&mut out, &mut para);
                out.push_str("<blockquote>\n");
                quote = true;
            }
            para.push(rest.trim_start().to_string());
            continue;
        }
        // 箇条書き・番号付き。段は行頭の空白で決まる。
        if let Some((kind, indent, text)) = item(trimmed) {
            flush_para(&mut out, &mut para);
            if quote {
                out.push_str("</blockquote>\n");
                quote = false;
            }
            lists.begin_item(&mut out, kind, indent, text);
            continue;
        }
        // ただの段落。**箇条書きの続きは畳まない**——`- 長い行` の折り返しを
        // 段落として起こすと、箇条書きの外へ落ちる。同じ項目の続きとして繋ぐ。
        if lists.is_open() {
            lists.continue_item(trimmed.trim_start());
            continue;
        }
        para.push(trimmed.trim_start().to_string());
    }
    if let Some((_, code)) = fence {
        let _ = writeln!(out, "<pre><code>{}</code></pre>", escape(&code));
    }
    flush(&mut out, &mut para, &mut lists, &mut quote);
    out
}

/// 囲みコードの印の本数。3本以上のバッククォートで始まる行だけ数える。
fn backticks(line: &str) -> Option<usize> {
    let ticks = line.trim_start().chars().take_while(|c| *c == '`').count();
    (ticks >= 3).then_some(ticks)
}

/// 表の行か。GFM は行頭のパイプを必須にしないが、ここでは**要る**ものとして
/// 扱う——先頭のパイプが無いと、ただの文章に混ざったパイプを拾ってしまう。
fn is_table_row(line: &str) -> bool {
    let line = line.trim();
    line.starts_with('|') && line.len() > 1
}

/// `|---|:--:|` のような仕切り行か。
fn is_table_divider(line: &str) -> bool {
    let line = line.trim();
    if !line.starts_with('|') {
        return false;
    }
    let cells = split_row(line);
    !cells.is_empty()
        && cells
            .iter()
            .all(|cell| cell.contains('-') && cell.chars().all(|c| c == '-' || c == ':'))
}

/// 行をセルへ割る。前後のパイプは飾りなので落とす。
fn split_row(line: &str) -> Vec<String> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line.strip_suffix('|').unwrap_or(line);
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // `\|` は区切りではなく字としてのパイプ。
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                cell.push('|');
            }
            '|' => cells.push(std::mem::take(&mut cell).trim().to_string()),
            _ => cell.push(c),
        }
    }
    cells.push(cell.trim().to_string());
    cells
}

/// 仕切りから列ごとの寄せを読む。
fn alignments(divider: &str) -> Vec<&'static str> {
    split_row(divider)
        .iter()
        .map(|cell| match (cell.starts_with(':'), cell.ends_with(':')) {
            (true, true) => "center",
            (false, true) => "right",
            _ => "left",
        })
        .collect()
}

/// 見出し行・仕切り・残りの行から表を組む。
fn table(header: &str, divider: &str, rows: &[&str]) -> String {
    let aligns = alignments(divider);
    let align_of = |index: usize| aligns.get(index).copied().unwrap_or("left");
    let mut out = String::from("<table>\n<thead><tr>");
    for (index, cell) in split_row(header).iter().enumerate() {
        let _ = write!(
            out,
            "<th style=\"text-align:{}\">{}</th>",
            align_of(index),
            inline(cell)
        );
    }
    out.push_str("</tr></thead>\n<tbody>\n");
    for row in rows {
        out.push_str("<tr>");
        for (index, cell) in split_row(row).iter().enumerate() {
            let _ = write!(
                out,
                "<td style=\"text-align:{}\">{}</td>",
                align_of(index),
                inline(cell)
            );
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</tbody>\n</table>\n");
    out
}

/// 開きっぱなしの塊を全部閉じる。
fn flush(out: &mut String, para: &mut Vec<String>, lists: &mut Lists, quote: &mut bool) {
    flush_para(out, para);
    lists.close(out);
    if *quote {
        out.push_str("</blockquote>\n");
        *quote = false;
    }
}

/// 溜まっている段落を1つ吐く。
fn flush_para(out: &mut String, para: &mut Vec<String>) {
    if para.is_empty() {
        return;
    }
    let _ = writeln!(out, "<p>{}</p>", inline(&para.join(" ")));
    para.clear();
}

/// 開いている一覧と、書きかけの項目。
///
/// 段は**字下げの幅を数えず、親より深いかどうかで積む**——`- ` の子は2字、
/// `1. ` の子は3字と md では幅が揃わないので、幅を段数に割ると同じ入れ子が
/// 深さ1と深さ2に散る。
#[derive(Default)]
struct Lists {
    /// (タグ, その段の行頭字下げ)。
    open: Vec<(&'static str, usize)>,
    /// 書きかけの項目の中身。折り返した行を同じ項目へ畳むため閉じるまで持つ。
    item: Option<String>,
}

impl Lists {
    fn is_open(&self) -> bool {
        !self.open.is_empty()
    }

    /// 新しい項目を始める。段の開け閉めもここで済ませる。
    fn begin_item(&mut self, out: &mut String, kind: &'static str, indent: usize, text: &str) {
        self.flush_item(out);
        while self.open.len() > 1 && self.open.last().is_some_and(|&(_, at)| indent < at) {
            self.pop(out);
        }
        match self.open.last().copied() {
            Some((_, at)) if indent > at => self.push(out, kind, indent),
            // 同じ段で種類が変わったら開き直す(`-` の並びが `1.` に変わる形)。
            Some((tag, _)) if tag != kind => {
                self.pop(out);
                self.push(out, kind, indent);
            }
            Some(_) => {}
            None => self.push(out, kind, indent),
        }
        self.item = Some(text.to_string());
    }

    /// 折り返した行を、書きかけの項目へ繋ぐ。
    fn continue_item(&mut self, text: &str) {
        match self.item.as_mut() {
            Some(item) => {
                item.push(' ');
                item.push_str(text);
            }
            None => self.item = Some(text.to_string()),
        }
    }

    fn close(&mut self, out: &mut String) {
        self.flush_item(out);
        while !self.open.is_empty() {
            self.pop(out);
        }
    }

    fn flush_item(&mut self, out: &mut String) {
        if let Some(item) = self.item.take() {
            out.push_str(&item_html(&item));
        }
    }

    fn push(&mut self, out: &mut String, kind: &'static str, indent: usize) {
        self.open.push((kind, indent));
        let _ = writeln!(out, "<{kind}>");
    }

    fn pop(&mut self, out: &mut String) {
        if let Some((tag, _)) = self.open.pop() {
            let _ = writeln!(out, "</{tag}>");
        }
    }
}

/// 項目1つ。`- [ ]` / `- [x]` は札にする——字のままだと本文と混ざって
/// 済み・未済が読み取れない。
fn item_html(text: &str) -> String {
    for (mark, checked) in [("[ ] ", ""), ("[x] ", " checked"), ("[X] ", " checked")] {
        if let Some(rest) = text.strip_prefix(mark) {
            return format!(
                "<li class=\"task\"><input type=\"checkbox\" disabled{checked}>{}</li>\n",
                inline(rest)
            );
        }
    }
    format!("<li>{}</li>\n", inline(text))
}

/// 畳む節の生 HTML。**属性なしの完全一致だけ**通す——`<details onclick=…>` の
/// ような札を、こちらが作った頁に貼らせない。
fn html_block(line: &str) -> Option<String> {
    match line {
        "<details>" | "</details>" | "<br>" | "<br/>" | "<br />" => Some(format!("{line}\n")),
        _ => {
            let inner = line
                .strip_prefix("<summary>")
                .and_then(|rest| rest.strip_suffix("</summary>"))?;
            Some(format!("<summary>{}</summary>\n", inline(inner)))
        }
    }
}

/// `#` の並びから見出しの深さと中身。
fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    // `#見出し` は見出しではない(md の作法)。
    let text = rest.strip_prefix(' ')?;
    Some((hashes, text.trim()))
}

/// 区切り線か(`---` / `***` / `___`)。
fn is_rule(line: &str) -> bool {
    for c in ['-', '*', '_'] {
        if line.len() >= 3 && line.chars().all(|got| got == c) {
            return true;
        }
    }
    false
}

/// 箇条書きの1行か。返すのは(タグ, 行頭の字下げ, 中身)。段に割るのは
/// [`Lists::begin_item`] の仕事——字下げの幅は段数に比例しない。
fn item(line: &str) -> Option<(&'static str, usize, &str)> {
    let rest = line.trim_start();
    // タブは幅4字として数える(混在した md で段が逆さになるのを防ぐ)。
    let indent = line[..line.len() - rest.len()]
        .chars()
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum();
    for mark in ["- ", "* ", "+ "] {
        if let Some(text) = rest.strip_prefix(mark) {
            return Some(("ul", indent, text.trim()));
        }
    }
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && let Some(text) = rest[digits..].strip_prefix(". ")
    {
        return Some(("ol", indent, text.trim()));
    }
    None
}

/// 行の中の記法。**コードを先に抜いて札に置き換える**——`` `**` `` のような字が
/// コードの中に居るとき太字として食われるのを防ぎ、かつ
/// ``**`topic/issue-405`**`` のように**太字がコードを跨ぐ**書き方を通すため。
/// 抜いて捨てると印だけが残り、利用者の md で一番よく出るこの形が生の `**` で出る。
fn inline(text: &str) -> String {
    // 札の印は本文に出ない制御文字。万一混ざっていたら落としてから使う。
    let text = text.replace(MARKER, "");
    let mut codes: Vec<String> = Vec::new();
    let mut stripped = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(open) = rest.find('`') {
        stripped.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('`') {
            Some(close) => {
                let _ = write!(stripped, "{MARKER}{}{MARKER}", codes.len());
                codes.push(escape(&after[..close]));
                rest = &after[close + 1..];
            }
            None => {
                // 閉じていないバッククォートはただの字。
                stripped.push_str(&rest[open..]);
                rest = "";
                break;
            }
        }
    }
    stripped.push_str(rest);
    // 札を戻す前に生 HTML の許しを与える——コードは札のままなので、
    // `` `<br>` `` と書いたコードがタグに化けない。
    restore_codes(&allow_bare_tags(&marks(&stripped)), &codes)
}

/// escape した札のうち、畳む節と改行だけ元に戻す。**属性が付いた札は戻さない**
/// ——`<details onclick=…>` は字のまま見せる。task md は
/// `<details>旧・…</details>` を1行に書くので、行の中でも通せないと字が出る。
fn allow_bare_tags(text: &str) -> String {
    let mut out = text.to_string();
    for tag in [
        "details", "/details", "summary", "/summary", "br", "br/", "br /",
    ] {
        if out.contains("&lt;") {
            out = out.replace(&format!("&lt;{tag}&gt;"), &format!("<{tag}>"));
        }
    }
    out
}

/// コードを預けた札の印。escape も印の探し方も素通りする字を使う。
const MARKER: char = '\u{0}';

/// 預けたコードを札の位置へ戻す。
fn restore_codes(text: &str, codes: &[String]) -> String {
    if codes.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find(MARKER) {
        out.push_str(&rest[..open]);
        let after = &rest[open + MARKER.len_utf8()..];
        let Some(close) = after.find(MARKER) else {
            break;
        };
        match after[..close]
            .parse::<usize>()
            .ok()
            .and_then(|at| codes.get(at))
        {
            Some(code) => {
                let _ = write!(out, "<code>{code}</code>");
            }
            None => out.push_str(&after[..close]),
        }
        rest = &after[close + MARKER.len_utf8()..];
    }
    out.push_str(rest);
    out
}

/// コードを預けた後の、太字・斜体・打ち消し・リンク。
fn marks(text: &str) -> String {
    let text = links(text);
    let text = wrap(&text, "**", "strong");
    let text = wrap(&text, "~~", "del");
    wrap(&text, "*", "em")
}

/// `[表示](行き先)` を `<a>` に。行き先は `http`/`https` だけ通す
/// ——`javascript:` の類を、こちらが作った札に張らせない。
fn links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(mid) = rest[open..].find("](") else {
            break;
        };
        let Some(close) = rest[open + mid..].find(')') else {
            break;
        };
        let label = &rest[open + 1..open + mid];
        let href = &rest[open + mid + 2..open + mid + close];
        out.push_str(&escape(&rest[..open]));
        let lower = href.trim().to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            let _ = write!(
                out,
                "<a href=\"{}\">{}</a>",
                escape(href.trim()),
                escape(label)
            );
        } else {
            // 通さない行き先は、書かれていた字をそのまま見せる。
            let _ = write!(out, "{}", escape(&rest[open..open + mid + close + 1]));
        }
        rest = &rest[open + mid + close + 1..];
    }
    out.push_str(&escape(rest));
    out
}

/// 同じ印で挟まれた区間をタグで包む。**もう escape 済みの字を受ける**。
fn wrap(text: &str, mark: &str, tag: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find(mark) {
        let after = &rest[open + mark.len()..];
        let Some(close) = after.find(mark) else {
            break;
        };
        if close == 0 {
            // `****` のような空の区間は印のまま置く。
            out.push_str(&rest[..open + mark.len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..open]);
        let _ = write!(out, "<{tag}>{}</{tag}>", &after[..close]);
        rest = &after[close + mark.len()..];
    }
    out.push_str(rest);
    out
}

/// HTML に出してよい形へ。
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pipe_table_becomes_a_real_table() {
        let got = body("| 名 | 数 |\n|:---|---:|\n| あ | 1 |\n| い | 2 |\n");
        assert!(got.contains("<table>"), "{got}");
        assert!(
            got.contains(r#"<th style="text-align:left">名</th>"#),
            "{got}"
        );
        // 仕切りの `:` から寄せを読む。
        assert!(
            got.contains(r#"<th style="text-align:right">数</th>"#),
            "{got}"
        );
        assert!(
            got.contains(r#"<td style="text-align:right">2</td>"#),
            "{got}"
        );
        // 見出しと中身が別の塊に入る。
        assert!(got.contains("<thead>") && got.contains("<tbody>"), "{got}");
    }

    #[test]
    fn pipes_without_a_divider_stay_a_paragraph() {
        // 仕切りが無ければ表にしない——コマンド例のパイプを壊さないため。
        let got = body("| これは表ではない |\nただの続き\n");
        assert!(!got.contains("<table>"), "{got}");
    }

    #[test]
    fn an_escaped_pipe_stays_inside_the_cell() {
        let got = body("| 記号 | 意味 |\n|---|---|\n| a \\| b | または |\n");
        assert!(
            got.contains("<td style=\"text-align:left\">a | b</td>"),
            "{got}"
        );
    }

    #[test]
    fn a_heading_and_a_paragraph_come_out_as_tags() {
        let got = body("# 題\n\n本文の1行目\n本文の2行目\n");
        assert!(got.contains("<h1>題</h1>"), "{got}");
        // 続きの行は同じ段落へ畳む。
        assert!(got.contains("<p>本文の1行目 本文の2行目</p>"), "{got}");
    }

    #[test]
    fn a_hash_without_a_space_is_not_a_heading() {
        let got = body("#タグのつもり\n");
        assert!(!got.contains("<h1"), "{got}");
        assert!(got.contains("<p>#タグのつもり</p>"), "{got}");
    }

    #[test]
    fn nested_bullets_open_and_close_in_order() {
        let got = body("- 上\n  - 中\n  - 中2\n- 上2\n");
        assert_eq!(got.matches("<ul>").count(), 2, "{got}");
        assert_eq!(got.matches("</ul>").count(), 2, "{got}");
        assert_eq!(got.matches("<li>").count(), 4, "{got}");
        // 閉じ忘れると後ろの本文まで一覧の中に入る。
        assert!(got.trim_end().ends_with("</ul>"), "{got}");
    }

    #[test]
    fn a_numbered_list_is_its_own_kind() {
        let got = body("1. 一\n2. 二\n");
        assert!(got.contains("<ol>") && got.contains("</ol>"), "{got}");
        assert_eq!(got.matches("<li>").count(), 2, "{got}");
    }

    #[test]
    fn a_fenced_block_is_not_interpreted_at_all() {
        let got = body("```\n# これは見出しではない\n**太字でもない**\n```\n");
        assert!(got.contains("<pre><code>"), "{got}");
        assert!(got.contains("# これは見出しではない"), "{got}");
        assert!(!got.contains("<h1"), "{got}");
        assert!(!got.contains("<strong>"), "{got}");
    }

    #[test]
    fn bold_italic_and_code_survive_together() {
        let got = inline("**太字**と*斜体*と`コード`");
        assert_eq!(
            got,
            "<strong>太字</strong>と<em>斜体</em>と<code>コード</code>"
        );
        // コードの中の印は字のまま。
        assert_eq!(inline("`**`"), "<code>**</code>");
    }

    #[test]
    fn only_http_links_become_anchors() {
        assert!(
            inline("[見る](https://example.com/x)")
                .contains("<a href=\"https://example.com/x\">見る</a>")
        );
        // `javascript:` の類を、こちらが作った札に張らせない。
        let got = inline("[押して](javascript:alert(1))");
        assert!(!got.contains("<a "), "{got}");
        assert!(got.contains("javascript"), "書かれていた字は見せる: {got}");
    }

    #[test]
    fn the_text_is_escaped_before_anything_else() {
        let got = inline("<script>alert(\"x\")</script> & そのまま");
        assert!(!got.contains("<script>"), "{got}");
        assert!(got.contains("&lt;script&gt;"), "{got}");
        assert!(got.contains("&amp;"), "{got}");
    }

    #[test]
    fn a_quote_block_opens_and_closes() {
        let got = body("> 引いた行\n> 続き\n\n外\n");
        assert_eq!(got.matches("<blockquote>").count(), 1, "{got}");
        assert_eq!(got.matches("</blockquote>").count(), 1, "{got}");
        assert!(got.contains("外"), "{got}");
    }

    #[test]
    fn bold_survives_a_code_span_inside_it() {
        // 利用者の md で一番多い形。抜いたコードを札にせず捨てると `**` が生で出る。
        let got = inline("ブランチ **`topic/issue-405`** を見る");
        assert_eq!(
            got,
            "ブランチ <strong><code>topic/issue-405</code></strong> を見る"
        );
        assert!(!got.contains("**"), "{got}");
        // 太字の途中にコードが挟まる形も跨ぐ。
        assert_eq!(
            inline("**`a` と `b` の両方**"),
            "<strong><code>a</code> と <code>b</code> の両方</strong>"
        );
    }

    #[test]
    fn front_matter_becomes_a_meta_card() {
        let got = to_html("---\nID: 861\n親: \n状態: 進行\n---\n\n# 題\n", "861.md");
        assert!(got.contains("<dl class=\"meta\">"), "{got}");
        assert!(got.contains("<dt>ID</dt><dd>861</dd>"), "{got}");
        // 空欄も見せる(「無い」も読み取れる情報)。
        assert!(
            got.contains("<dt>親</dt><dd class=\"empty\">—</dd>"),
            "{got}"
        );
        // メタが区切り線と潰れた1段落に化けない。
        assert!(!got.contains("<hr>"), "{got}");
        assert!(!got.contains("<p>ID: 861"), "{got}");
        assert!(got.contains("<h1>題</h1>"), "{got}");
    }

    #[test]
    fn an_unclosed_leading_rule_is_still_a_rule() {
        // 閉じの `---` が無ければメタではない——本文の区切り線を食わない。
        assert!(front_matter("---\n本文\n").is_none());
        let got = to_html("---\n本文\n", "x.md");
        assert!(got.contains("<hr>"), "{got}");
        assert!(!got.contains("<dl class=\"meta\">"), "{got}");
    }

    #[test]
    fn a_wrapped_bullet_stays_one_item() {
        let got = body("- 長い項目の\n  折り返した続き\n- 次\n");
        assert_eq!(got.matches("<li>").count(), 2, "{got}");
        assert!(got.contains("<li>長い項目の 折り返した続き</li>"), "{got}");
    }

    #[test]
    fn indent_widths_do_not_change_the_depth() {
        // `1. ` の子は3字下げになる。幅を段数に割ると深さ1と2に散る。
        let got = body("1. 親\n   - 子\n   - 子2\n");
        assert_eq!(got.matches("<ul>").count(), 1, "{got}");
        assert_eq!(got.matches("<ol>").count(), 1, "{got}");
        assert_eq!(got.matches("<li>").count(), 3, "{got}");
    }

    #[test]
    fn a_task_item_becomes_a_checkbox() {
        let got = body("- [ ] 未\n- [x] 済\n");
        assert_eq!(
            got.matches("<input type=\"checkbox\" disabled").count(),
            2,
            "{got}"
        );
        assert!(got.contains("disabled checked>済"), "{got}");
        assert!(!got.contains("[ ]"), "{got}");
    }

    #[test]
    fn bare_tags_pass_inside_a_line_but_code_keeps_them_as_text() {
        // task md は畳む節を1行に書く。
        let got = inline("<details>旧・次の一歩: 済み</details>");
        assert!(
            got.starts_with("<details>") && got.ends_with("</details>"),
            "{got}"
        );
        assert_eq!(inline("上<br>下"), "上<br>下");
        // コードの中の札は字のまま——札を戻す順で守る。
        assert_eq!(inline("`<br>`"), "<code>&lt;br&gt;</code>");
    }

    #[test]
    fn a_deeper_fence_can_hold_a_shorter_one() {
        // 4本で開けた囲みの中の3本を、閉じと読まない。
        let got = body("````markdown\n```bash\necho x\n```\n````\n\n**外**\n");
        assert_eq!(got.matches("<pre>").count(), 1, "{got}");
        assert!(got.contains("```bash"), "{got}");
        assert!(got.contains("<strong>外</strong>"), "{got}");
    }

    #[test]
    fn strikethrough_becomes_del() {
        assert_eq!(inline("~~消した~~あと"), "<del>消した</del>あと");
    }

    #[test]
    fn only_bare_details_tags_pass_through_as_html() {
        let got = body("<details>\n<summary>**畳んだ題**</summary>\n\n中身\n</details>\n");
        assert!(
            got.contains("<details>") && got.contains("</details>"),
            "{got}"
        );
        // summary の中は md として読む。
        assert!(
            got.contains("<summary><strong>畳んだ題</strong></summary>"),
            "{got}"
        );
        assert!(got.contains("<p>中身</p>"), "{got}");
        // 属性が付いた札は通さない——字として見せる。
        let attacked = body("<details onclick=\"x\">\n");
        assert!(!attacked.contains("<details"), "{attacked}");
        assert!(attacked.contains("&lt;details"), "{attacked}");
    }

    #[test]
    fn the_whole_page_carries_the_title_and_the_board_colours() {
        let got = to_html("# 題\n", "棚卸し.md");
        assert!(got.starts_with("<!DOCTYPE html>"), "{got}");
        assert!(got.contains("<title>棚卸し.md</title>"), "{got}");
        // 地色は窓の地。新しい色は作らない。
        let window = crate::palette::surface_window();
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let base = format!(
            "#{:02x}{:02x}{:02x}",
            byte(window.r),
            byte(window.g),
            byte(window.b)
        );
        assert!(got.contains(&format!("html{{background:{base}}}")), "{got}");
    }
}

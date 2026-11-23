use super::*;

fn plain(text: &str) -> Inline {
    Inline { spans: vec![Span { text: text.into(), style: Style::default(), link: None }] }
}

fn text_of(inline: &Inline) -> String {
    inline.spans.iter().map(|s| s.text.as_str()).collect()
}

fn internal(target: &str, heading: Option<&str>, block: Option<&str>) -> Link {
    Link::Internal { target: target.into(), heading: heading.map(Into::into), block: block.map(Into::into) }
}

#[test]
fn lists_with_task_lines_after_crlf_frontmatter() {
    let src = "---\r\ntitle: x\r\n---\r\n# Title\r\n\r\n- [ ] one\r\n- [x] two\r\n  - [ ] nested\r\n- plain\r\n\r\n3. three\r\n";
    let mut blocks = parse(src);
    assert_eq!(blocks.remove(0), Block::Properties(vec![Property { key: "title".into(), values: vec![plain("x")] }]));
    assert_eq!(blocks[0], Block::Heading { level: 1, text: plain("Title") });
    let Block::List { start: None, items } = &blocks[1] else { panic!("{blocks:?}") };
    assert_eq!(items[0], Item { task: Some(Task { done: false, line: 5 }), blocks: vec![Block::Paragraph(plain("one"))] });
    assert_eq!(items[1].task, Some(Task { done: true, line: 6 }));
    let Block::List { items: nested, .. } = &items[1].blocks[1] else { panic!("{:?}", items[1]) };
    assert_eq!(nested[0].task, Some(Task { done: false, line: 7 }));
    assert_eq!(items[2], Item { task: None, blocks: vec![Block::Paragraph(plain("plain"))] });
    assert!(matches!(&blocks[2], Block::List { start: Some(3), items } if items.len() == 1));
    assert_eq!(blocks.len(), 3);
}

#[test]
fn properties_with_lists_links_and_tags() {
    let src = "---\ntitle: Alpha\ntags: [project/alpha, \"#work\"]\nup: \"[[Parent#Sec|the parent]]\"\n\
               estimate: 12.5\ndone: false\nempty:\nowners:\n  - Ann\n  - \"[[Bob]]\"\n---\nbody\n";
    let blocks = parse(src);
    let Block::Properties(props) = &blocks[0] else { panic!("{blocks:?}") };
    let keys: Vec<&str> = props.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["title", "tags", "up", "estimate", "done", "empty", "owners"]);
    let tag = |t: &str| Inline { spans: vec![Span { text: t.into(), style: Style { tag: true, ..Style::default() }, link: None }] };
    assert_eq!(props[1].values, [tag("#project/alpha"), tag("#work")]);
    let link = |text: &str, target: &str, heading: Option<&str>| Span {
        text: text.into(),
        style: Style::default(),
        link: Some(internal(target, heading, None)),
    };
    assert_eq!(props[2].values, [Inline { spans: vec![link("the parent", "Parent", Some("Sec"))] }]);
    assert_eq!((props[3].values[0].clone(), props[4].values[0].clone()), (plain("12.5"), plain("false")));
    assert!(props[5].values.is_empty());
    assert_eq!(props[6].values, [plain("Ann"), Inline { spans: vec![link("Bob", "Bob", None)] }]);
    assert_eq!(blocks[1], Block::Paragraph(plain("body")));
    assert_eq!(parse("---\n: : [\n---\nx")[0], Block::Paragraph(plain("x")), "broken YAML shows nothing");
}

#[test]
fn callouts() {
    let src = "> [!NOTE]\n> Body *text*\n\n> [!tip]- Read **this** first\n> hidden\n>\n> - [ ] task\n\n\
               > [!faq]+\n\n> [!warning] Title\r\n> line\r\n\n> plain quote\n> [!not] a callout here\n\n\
               - item\n  > [!bug] In a list\n";
    let blocks = parse(src);
    let callout = |kind: &str, title: &str, fold, key: &str, blocks| Block::Callout {
        kind: kind.into(),
        title: plain(title),
        fold,
        key: key.into(),
        blocks,
    };
    let em = Span { text: "text".into(), style: Style { emphasis: true, ..Style::default() }, link: None };
    let body = Inline { spans: vec![Span { text: "Body ".into(), style: Style::default(), link: None }, em] };
    assert_eq!(blocks[0], callout("note", "Note", None, "note|Note|0", vec![Block::Paragraph(body)]));
    let Block::Callout { title, fold, key, blocks: inner, .. } = &blocks[1] else { panic!("{blocks:?}") };
    let strong = Style { strong: true, ..Style::default() };
    let shown: Vec<(&str, Style)> = title.spans.iter().map(|s| (s.text.as_str(), s.style)).collect();
    assert_eq!(shown, [("Read ", Style::default()), ("this", strong), (" first", Style::default())], "titles keep their formatting");
    assert_eq!((*fold, key.as_str()), (Some(false), "tip|Read **this** first|0"));
    assert_eq!(inner[0], Block::Paragraph(plain("hidden")));
    assert!(matches!(&inner[1], Block::List { items, .. } if items[0].task == Some(Task { done: false, line: 6 })));
    assert_eq!(blocks[2], callout("faq", "Faq", Some(true), "faq|Faq|0", vec![]));
    assert_eq!(blocks[3], callout("warning", "Title", None, "warning|Title|0", vec![Block::Paragraph(plain("line"))]));
    assert!(matches!(&blocks[4], Block::Quote(_)), "the marker counts on the first line only");
    let Block::List { items, .. } = &blocks[5] else { panic!("{blocks:?}") };
    assert!(matches!(&items[0].blocks[1], Block::Callout { kind, .. } if kind == "bug"));

    // Keys count callouts alike, and do not move when lines are added above.
    let keys = |src: &str| -> Vec<String> {
        parse(src).into_iter().filter_map(|b| match b { Block::Callout { key, .. } => Some(key), _ => None }).collect()
    };
    let twice = "> [!tip]- Same\n> a\n\n> [!tip]- Same\n> b\n\n> [!tip]- [[Other]] #x\n";
    assert_eq!(keys(twice), ["tip|Same|0", "tip|Same|1", "tip|[[Other]] #x|0"]);
    assert_eq!(keys(&format!("# New heading\n\nNew text\n\n{twice}")), keys(twice));
    let Block::Callout { title, .. } = &parse("> [!tip] See [[Other|it]] #x\n")[0] else { panic!() };
    assert!(title.spans.iter().any(|s| s.text == "it" && s.link.is_some()) && title.spans.iter().any(|s| s.style.tag));
    let Block::Callout { title, .. } = &parse("> [!tip] 1. Step\n")[0] else { panic!() };
    assert_eq!(title, &plain("1. Step"), "a title that is not a paragraph stays as written");
}

#[test]
fn highlights() {
    let blocks = parse("a ==marked *em [[N]]*== b ==one== `==c==` and ==open\n\n# Head ==h==");
    let Block::Paragraph(p) = &blocks[0] else { panic!("{blocks:?}") };
    let shown: Vec<(&str, bool, bool)> = p.spans.iter().map(|s| (s.text.as_str(), s.style.highlight, s.style.emphasis)).collect();
    assert_eq!(
        shown,
        [
            ("a ", false, false),
            ("marked ", true, false),
            ("em ", true, true),
            ("N", true, true),
            (" b ", false, false),
            ("one", true, false),
            (" ", false, false),
            ("==c==", false, false),
            (" and ==open", false, false),
        ]
    );
    let Block::Heading { text, .. } = &blocks[1] else { panic!("{blocks:?}") };
    assert!(text.spans.iter().any(|s| s.text == "h" && s.style.highlight));
}

#[test]
fn loose_lists_and_a_byte_order_mark() {
    let blocks = parse("- [ ] loose\n\n  more\n- [x] two\n");
    let Block::List { items, .. } = &blocks[0] else { panic!("{blocks:?}") };
    assert_eq!(items[0].task, Some(Task { done: false, line: 0 }));
    assert_eq!(items[0].blocks, [Block::Paragraph(plain("loose")), Block::Paragraph(plain("more"))]);
    assert_eq!(items[1].task, Some(Task { done: true, line: 3 }));

    let blocks = parse("\u{feff}# Bom\n- [ ] t\n");
    assert_eq!(blocks[0], Block::Heading { level: 1, text: plain("Bom") });
    let Block::List { items, .. } = &blocks[1] else { panic!("{blocks:?}") };
    assert_eq!(items[0].task, Some(Task { done: false, line: 1 }));
}

#[test]
fn wikilinks_and_markdown_links() {
    let blocks = parse(
        "[[Note#Part#Sec]] [[Note|alias]] [[#Head]] [[Note#^blk]] [m](Other%20note.md#Sec%20one) \
         [e](https://e.com) <https://a.b> <me@x.org> [b](#^id) [none]()",
    );
    let Block::Paragraph(p) = &blocks[0] else { panic!("{blocks:?}") };
    let links: Vec<(&str, &Link)> =
        p.spans.iter().filter_map(|s| s.link.as_ref().map(|l| (s.text.as_str(), l))).collect();
    assert_eq!(
        links,
        [
            ("Note > Part > Sec", &internal("Note", Some("Part#Sec"), None)),
            ("alias", &internal("Note", None, None)),
            ("Head", &internal("", Some("Head"), None)),
            ("Note > ^blk", &internal("Note", None, Some("blk"))),
            ("m", &internal("Other note.md", Some("Sec one"), None)),
            ("e", &Link::External("https://e.com".into())),
            ("https://a.b", &Link::External("https://a.b".into())),
            ("me@x.org", &Link::External("mailto:me@x.org".into())),
            ("b", &internal("", None, Some("id"))),
        ]
    );
    assert!(text_of(p).ends_with(" none"), "an empty destination is no link");
}

#[test]
fn inline_styles_and_tags() {
    let blocks = parse("**bold [[A|b]]** *em* ~~st~~ `c #t` #tag and #12 [x #no](y)\nnext  \nline");
    let Block::Paragraph(p) = &blocks[0] else { panic!("{blocks:?}") };
    let strong = Style { strong: true, ..Style::default() };
    let shown: Vec<(&str, Style, bool)> = p.spans.iter().map(|s| (s.text.as_str(), s.style, s.link.is_some())).collect();
    assert_eq!(
        shown,
        [
            ("bold ", strong, false),
            ("b", strong, true),
            (" ", Style::default(), false),
            ("em", Style { emphasis: true, ..Style::default() }, false),
            (" ", Style::default(), false),
            ("st", Style { strike: true, ..Style::default() }, false),
            (" ", Style::default(), false),
            ("c #t", Style { code: true, ..Style::default() }, false),
            (" ", Style::default(), false),
            ("#tag", Style { tag: true, ..Style::default() }, false),
            (" and #12 ", Style::default(), false),
            ("x #no", Style::default(), true),
            (" next\nline", Style::default(), false),
        ]
    );
}

#[test]
fn embeds_split_paragraphs() {
    let blocks = parse(
        "Before ![[pic.png|300]] middle ![alt|200](img/a%20b.png) ![[Note#Sec]] ![](https://x/y.png) \
         ![[c.png|Caption|40x30]] ![[Note#^b]] after",
    );
    let embed = |target: &str, width, alt: &str| Embed {
        target: target.into(),
        heading: None,
        block: None,
        width,
        alt: alt.into(),
        external: false,
    };
    assert_eq!(
        blocks,
        [
            Block::Paragraph(plain("Before")),
            Block::Embed(embed("pic.png", Some(300), "")),
            Block::Paragraph(plain("middle")),
            Block::Embed(embed("img/a b.png", Some(200), "alt")),
            Block::Embed(Embed { heading: Some("Sec".into()), ..embed("Note", None, "") }),
            Block::Embed(Embed { external: true, ..embed("https://x/y.png", None, "") }),
            Block::Embed(embed("c.png", Some(40), "Caption")),
            Block::Embed(Embed { block: Some("b".into()), ..embed("Note", None, "") }),
            Block::Paragraph(plain("after")),
        ]
    );
}

#[test]
fn images_that_cannot_be_blocks() {
    let blocks = parse(
        "- item ![[a.png]] after\n\n# Head ![[x.png]]\n\n| ![i](p.png) | ![](q.png) |\n|---|---|\n\n\
         [![logo](l.png)](https://x.y)\n\n![300](plain.png)\n",
    );
    let Block::List { items, .. } = &blocks[0] else { panic!("{blocks:?}") };
    assert_eq!(items[0].blocks.len(), 3, "the embed splits the item's text: {:?}", items[0].blocks);
    assert!(matches!(&items[0].blocks[1], Block::Embed(e) if e.target == "a.png"));
    assert_eq!(blocks[1], Block::Heading { level: 1, text: plain("Head x.png") });
    let Block::Table { header, .. } = &blocks[2] else { panic!("{blocks:?}") };
    assert_eq!(header, &[plain("i"), plain("q.png")]);
    let Block::Paragraph(linked) = &blocks[3] else { panic!("{blocks:?}") };
    assert_eq!(linked.spans.len(), 1);
    assert_eq!((linked.spans[0].text.as_str(), &linked.spans[0].link), ("logo", &Some(Link::External("https://x.y".into()))));
    assert!(
        matches!(&blocks[4], Block::Embed(e) if e.alt == "300" && e.width.is_none()),
        "Markdown alt text needs `|` before a size"
    );
}

#[test]
fn code_views_quotes_tables_and_html() {
    let src = "intro\n\n```view\ntype: kanban\n```\n\n```rust title\nfn x(){}\n```\n\n    indented\n\n\
               ~~~view kanban\nbody\n~~~\n\n> quote\n> > nested\n\n---\n\n<div>html</div>\n\n\
               | a | b | c |\n|:--|--:|:-:|\n| `x` | y | |\n\n```\r\nr\r\nn\r\n```\r\n";
    let code = |language: Option<&str>, code: &str| Block::Code { language: language.map(Into::into), code: code.into() };
    let cell_x = Inline { spans: vec![Span { text: "x".into(), style: Style { code: true, ..Style::default() }, link: None }] };
    assert_eq!(
        parse(src),
        [
            Block::Paragraph(plain("intro")),
            Block::View { source: "type: kanban".into(), line: 2 },
            code(Some("rust"), "fn x(){}"),
            code(None, "indented"),
            Block::View { source: "body".into(), line: 12 },
            Block::Quote(vec![Block::Paragraph(plain("quote")), Block::Quote(vec![Block::Paragraph(plain("nested"))])]),
            Block::Rule,
            Block::Html("<div>html</div>".into()),
            Block::Table {
                alignments: vec![Align::Left, Align::Right, Align::Center],
                header: vec![plain("a"), plain("b"), plain("c")],
                rows: vec![vec![cell_x, plain("y"), Inline::default()]],
            },
            code(None, "r\nn"),
        ]
    );
}

#[test]
fn sections_and_blocks_for_embeds() {
    let note = "\u{feff}# Top\nintro\n## Part\npart text\n### Sub\nsub text\n## Other\nother ^para\n\n\
                | t |\n|---|\n| c |\n^tbl\n\nx^glued\n";
    let part = "## Part\npart text\n### Sub\nsub text";
    assert_eq!(section(note, "Part").as_deref(), Some(part));
    assert_eq!(section(note, "top#PART").as_deref(), Some(part), "a heading path names its last heading");
    assert_eq!(section(note, "Sub").as_deref(), Some("### Sub\nsub text"));
    assert!(section(note, "Top").unwrap().starts_with("# Top\nintro"), "no byte-order mark in the text");
    assert_eq!(section(note, "missing"), None);
    assert_eq!(section(note, "#"), None);

    assert_eq!(block(note, "para").as_deref(), Some("other"));
    assert_eq!(block(note, "tbl").as_deref(), Some("| t |\n|---|\n| c |"), "a marker on its own line");
    assert_eq!(block(note, "glued"), None, "a marker must stand apart from the text");
    assert_eq!(block(note, "nope"), None);
    assert_eq!(block(note, ""), None);
}

#[test]
fn task_references_embeds_and_queries_are_nodes() {
    let src = "---\nparent: \"[[proj-voice]]\"\n---\n# Standup with [[alice]]\n\n\
               - [In progress] [[fix-voice-reconnect]] and [[fix-voice-reconnect|again]]\n\
               - [[Spec#Goals]], [[#Local]], [doc](Other%20Note.md), [web](https://example.com)\n\n\
               ![[fix-voice-reconnect]]\n\n> [!note] About [[ops-rota]]\n> `[[not-a-link]]`\n\n\
               | Task |\n|---|\n| [[cell-task]] |\n\n```tasks\nstatus != done\nsort due\n```\n";
    let blocks = parse(src);
    assert_eq!(
        references(&blocks),
        ["proj-voice", "alice", "fix-voice-reconnect", "Other Note.md", "ops-rota", "cell-task"],
        "whole-document links, each once; not headings, same-note links, URLs or code"
    );
    assert!(blocks.contains(&Block::Embed(Embed {
        target: "fix-voice-reconnect".into(),
        heading: None,
        block: None,
        width: None,
        alt: String::new(),
        external: false,
    })));
    assert_eq!(blocks.last(), Some(&Block::TaskQuery { source: "status != done\nsort due".into(), line: 17 }));

    // The link's text stays as written: a piped alias, or the target.
    let Block::List { items, .. } = &blocks[2] else { panic!("{:?}", blocks[2]) };
    let Block::Paragraph(first) = &items[0].blocks[0] else { panic!() };
    let linked: Vec<(&str, &Link)> = first.spans.iter().filter_map(|s| Some((s.text.as_str(), s.link.as_ref()?))).collect();
    assert_eq!(
        linked,
        [("fix-voice-reconnect", &internal("fix-voice-reconnect", None, None)), ("again", &internal("fix-voice-reconnect", None, None))]
    );
}

#[test]
fn only_the_top_block_is_frontmatter() {
    // Further down, `---` pairs are a rule and a setext heading, as in Obsidian.
    let blocks = parse("Intro\n\n---\nSome paragraph\n---\n\nafter\n");
    assert_eq!(
        blocks,
        [
            Block::Paragraph(plain("Intro")),
            Block::Rule,
            Block::Heading { level: 2, text: plain("Some paragraph") },
            Block::Paragraph(plain("after")),
        ]
    );
    let blocks = parse("text\n\n---\nkey: v\n---\n");
    assert!(!blocks.iter().any(|b| matches!(b, Block::Properties(_))), "{blocks:?}");
    // Lines after frontmatter are still those of the file.
    let blocks = parse("\u{feff}---\ntitle: x\n---\n- [ ] one\n");
    let Block::List { items, .. } = &blocks[1] else { panic!("{blocks:?}") };
    assert_eq!(items[0].task, Some(Task { done: false, line: 3 }));
}

#[test]
fn escaped_pipe_in_a_table_link() {
    let blocks = parse("| a |\n|---|\n| [[Note\\|alias]] |\n");
    let Block::Table { rows, .. } = &blocks[0] else { panic!("{blocks:?}") };
    let linked: Vec<(&str, &Link)> = rows[0][0].spans.iter().filter_map(|s| Some((s.text.as_str(), s.link.as_ref()?))).collect();
    assert_eq!(linked, [("alias", &internal("Note", None, None))]);
}

#[test]
fn other_task_states_are_ticked_boxes() {
    let blocks = parse("- [/] half\n- [-] dropped\n  - [>] later\n1. [?] asked\n- [ ] open\n- [ab] not a task\n- [/]\n");
    let Block::List { items, .. } = &blocks[0] else { panic!("{blocks:?}") };
    assert_eq!(items[0], Item { task: Some(Task { done: true, line: 0 }), blocks: vec![Block::Paragraph(plain("half"))] });
    assert_eq!(items[1].task, Some(Task { done: true, line: 1 }));
    let Block::List { items: nested, .. } = &items[1].blocks[1] else { panic!("{:?}", items[1]) };
    assert_eq!(nested[0], Item { task: Some(Task { done: true, line: 2 }), blocks: vec![Block::Paragraph(plain("later"))] });
    let Block::List { items: numbered, .. } = &blocks[1] else { panic!("{blocks:?}") };
    assert_eq!(numbered[0].task, Some(Task { done: true, line: 3 }));
    let Block::List { items: rest, .. } = &blocks[2] else { panic!("{blocks:?}") };
    assert_eq!(rest[0].task, Some(Task { done: false, line: 4 }));
    assert_eq!(rest[1].task, None, "two characters are not a state");
    assert_eq!(rest[2], Item { task: Some(Task { done: true, line: 6 }), blocks: vec![] });
}

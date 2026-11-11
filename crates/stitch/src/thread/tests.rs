use super::*;
use crate::{Block, Pt, Step, StitchPlan};

const CSV: &str =
    "brand,code,name,hex,weight\nAcme,100,Snow,#ffffff,40wt\nAcme,200,\"Red, deep\",c81e28,\n\nAcme,300,\"The \"\"Navy\"\"\",#1e5ac8,40wt\n";

#[test]
fn a_csv_chart_reads_with_quotes_blank_lines_and_a_bare_hex() {
    let c = ThreadChart::parse_csv(CSV.as_bytes(), "Acme").unwrap();
    assert_eq!(c.name, "Acme");
    assert_eq!(c.threads.len(), 3);
    assert_eq!(c.threads[1].name, "Red, deep");
    assert_eq!(c.threads[1].rgb, [200, 30, 40]);
    assert_eq!(c.threads[1].weight, None);
    assert_eq!(c.threads[2].name, "The \"Navy\"");
    assert_eq!(c.threads[0].weight.as_deref(), Some("40wt"));
}

#[test]
fn columns_may_come_in_any_order_and_brand_defaults_to_the_chart_name() {
    let c = ThreadChart::parse_csv(b"\xEF\xBB\xBFHEX,Code\r\n#000000,1\r\n#ffffff,2", "Mine").unwrap();
    assert_eq!((c.threads[0].brand.as_str(), c.threads[1].code.as_str(), c.threads[1].rgb), ("Mine", "2", [255, 255, 255]));
}

#[test]
fn json_and_csv_charts_agree_and_json_round_trips() {
    let c = ThreadChart::parse_csv(CSV.as_bytes(), "Acme").unwrap();
    let j = ThreadChart::parse_json(c.to_json().as_bytes(), "x").unwrap();
    assert_eq!(c, j);
    assert_eq!(ThreadChart::parse(c.to_json().as_bytes(), "x").unwrap(), c);
    assert_eq!(ThreadChart::parse(CSV.as_bytes(), "Acme").unwrap(), c);
    let s = starter_chart();
    assert_eq!(ThreadChart::parse_json(s.to_json().as_bytes(), "").unwrap(), s);
}

#[test]
fn bad_charts_say_what_and_where() {
    let e = |b: &str| ThreadChart::parse(b.as_bytes(), "t").unwrap_err();
    assert!(e("").contains("empty"));
    assert!(e("code,name\n1,a").contains("`hex`"));
    assert!(e("code,hex\n1,#12345").contains("line 2") && e("code,hex\n1,#12345").contains("not a colour"));
    assert!(e("code,hex\n1,#112233,x").contains("3 fields"));
    assert!(e("code,hex\n1,#112233\n1,#445566").contains("line 3") && e("code,hex\n1,#112233\n1,#445566").contains("twice"));
    assert!(e("code,hex\n,#112233").contains("code is empty"));
    assert!(e("code,hex\n\"1,#112233").contains("never closed"));
    assert!(e("code,hex\n1,#11\u{1}2233").contains("not a colour"));
    assert!(e("code,hex\n").contains("no threads"));
    assert!(e(&format!("code,hex\n{},#112233", "x".repeat(MAX_CODE + 1))).contains("longer than"));
    assert!(e("code,name,hex\n1,a\u{7}b,#112233").contains("control character"));
    assert!(e("{\"threads\": 3}").contains("`threads` list"));
    assert!(e("{\"format\": \"other\", \"threads\": []}").contains("format"));
    assert!(e("{\"threads\": [{\"code\": \"1\", \"hex\": 5}]}").contains("must be text"));
    assert!(e("{\"threads\": [{\"code\": \"1\", \"hex\": \"zzzzzz\"}]}").contains("thread 1"));
    assert!(e("{\"threads\": []}").contains("no threads"));
    assert!(ThreadChart::parse(&[0xff, 0xfe, 0x00], "t").unwrap_err().contains("UTF-8"));
}

#[test]
fn charts_past_the_caps_are_refused() {
    let mut many = String::from("code,hex\n");
    for i in 0..=MAX_CHART_THREADS {
        many.push_str(&format!("{i},#000000\n"));
    }
    assert!(ThreadChart::parse_csv(many.as_bytes(), "t").unwrap_err().contains("more than"));
    let big = vec![b'a'; MAX_CHART_BYTES + 1];
    assert!(ThreadChart::parse_csv(&big, "t").unwrap_err().contains("larger"));
    assert!(ThreadChart::parse_csv(format!("code,hex\n1,{}", "9".repeat(5000)).as_bytes(), "t").is_err());
    assert!(ThreadChart::parse_csv(b"code,hex\n1,#000000", &"n".repeat(MAX_CHART_NAME + 1)).is_err());
}

#[test]
fn the_starter_set_is_24_distinct_original_colours() {
    let s = starter_chart();
    assert_eq!(s.threads.len(), 24);
    assert!(s.threads.iter().all(|t| t.brand == "VectorCraft" && !t.name.is_empty()));
    let mut codes: Vec<_> = s.threads.iter().map(|t| t.code.clone()).collect();
    codes.dedup();
    assert_eq!(codes.len(), 24);
    for (i, a) in s.threads.iter().enumerate() {
        for b in s.threads.iter().skip(i + 1) {
            assert!(delta_e(a.rgb, b.rgb) > 5.0, "{} and {} are too alike", a.name, b.name);
        }
    }
    let red = s.threads.iter().find(|t| t.name == "Red").unwrap();
    assert!(red.rgb[0] > 200 && red.rgb[1] < 60 && red.rgb[2] < 60, "{:?}", red.rgb);
}

#[test]
fn lab_is_the_standard_one() {
    let w = lab([255, 255, 255]);
    assert!((w[0] - 100.0).abs() < 0.01 && w[1].abs() < 0.01 && w[2].abs() < 0.01, "{w:?}");
    assert!(lab([0, 0, 0])[0].abs() < 1e-9);
    // sRGB red is about (53.2, 80.1, 67.2).
    let r = lab([255, 0, 0]);
    assert!((r[0] - 53.24).abs() < 0.1 && (r[1] - 80.09).abs() < 0.2 && (r[2] - 67.20).abs() < 0.2, "{r:?}");
    assert_eq!(delta_e([10, 20, 30], [10, 20, 30]), 0.0);
}

#[test]
fn the_nearest_thread_is_found_and_ties_go_to_the_first() {
    let s = starter_chart();
    let m = s.nearest([250, 20, 20]).unwrap();
    assert_eq!(s.threads[m.index].name, "Red");
    assert!(m.delta_e > 0.0 && m.delta_e < 15.0);
    let exact = s.nearest([0, 0, 0]).unwrap();
    assert_eq!((s.threads[exact.index].name.as_str(), exact.delta_e), ("Black", 0.0));
    let twin = ThreadChart {
        name: "t".into(),
        threads: vec![
            ThreadSpec { brand: "b".into(), code: "1".into(), name: String::new(), rgb: [9, 9, 9], weight: None },
            ThreadSpec { brand: "b".into(), code: "2".into(), name: String::new(), rgb: [9, 9, 9], weight: None },
        ],
    };
    assert_eq!(twin.nearest([0, 0, 0]).map(|m| m.index), Some(0));
    assert_eq!(ThreadChart { name: "e".into(), threads: vec![] }.nearest([0, 0, 0]), None);
}

fn plan(colours: &[[u8; 3]]) -> StitchPlan {
    StitchPlan {
        name: String::new(),
        blocks: colours
            .iter()
            .map(|&rgb| Block {
                thread: Thread { rgb, ..Default::default() },
                steps: vec![Step::Stitch(Pt { x: 0, y: 0 }), Step::Stitch(Pt { x: 10, y: 0 }), Step::Trim],
            })
            .collect(),
    }
}

#[test]
fn match_plan_gives_one_match_per_block_and_a_spec_makes_a_thread() {
    let s = starter_chart();
    let ms = s.match_plan(&plan(&[[255, 0, 0], [0, 0, 0]]));
    assert_eq!(ms.len(), 2);
    let t = s.threads[ms[1].unwrap().index].thread();
    assert_eq!((t.brand.as_deref(), t.code.as_deref(), t.name.as_deref()), (Some("VectorCraft"), Some("VC01"), Some("Black")));
}

#[test]
fn needles_follow_sewing_order_and_a_repeated_colour_keeps_its_needle() {
    let (r, g, b) = ([255, 0, 0], [0, 255, 0], [0, 0, 255]);
    let n = assign_needles(&plan(&[r, g, r, b, g]), Some(15));
    let needles: Vec<_> = n.uses.iter().map(|u| (u.needle, u.rgb, u.stitches, u.blocks.clone())).collect();
    assert_eq!(needles, vec![(Some(1), r, 4, vec![0, 2]), (Some(2), g, 4, vec![1, 4]), (Some(3), b, 2, vec![3])]);
    assert!(n.warnings.is_empty());
    assert_eq!(assign_needles(&StitchPlan::default(), Some(15)).uses.len(), 0);
}

#[test]
fn more_colours_than_needles_warns_and_leaves_the_rest_without() {
    let colours: Vec<[u8; 3]> = (0..4).map(|i| [i * 10, 0, 0]).collect();
    let n = assign_needles(&plan(&colours), Some(2));
    assert_eq!(n.uses.iter().map(|u| u.needle).collect::<Vec<_>>(), vec![Some(1), Some(2), None, None]);
    assert_eq!(n.warnings.len(), 1);
    assert!(
        n.warnings[0].contains("4 thread colours") && n.warnings[0].contains("2 needles") && n.warnings[0].contains("2 need"),
        "{:?}",
        n.warnings
    );
    let free = assign_needles(&plan(&colours), None);
    assert!(free.warnings.is_empty() && free.uses.iter().all(|u| u.needle.is_some()));
}

mod props {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn garbage_charts_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..600)) {
            let _ = ThreadChart::parse(&bytes, "t");
            let _ = ThreadChart::parse_csv(&bytes, "t");
            let _ = ThreadChart::parse_json(&bytes, "t");
        }

        #[test]
        fn csv_shaped_garbage_never_panics(rows in prop::collection::vec("[a-z0-9#,\" \\n\\r]{0,40}", 0..12)) {
            let text = format!("code,hex,name\n{}", rows.join("\n"));
            let _ = ThreadChart::parse(text.as_bytes(), "t");
        }

        #[test]
        fn the_nearest_thread_is_never_worse_than_any_other(rgb in any::<[u8; 3]>()) {
            let s = starter_chart();
            let m = s.nearest(rgb).unwrap();
            for t in &s.threads {
                prop_assert!(m.delta_e <= delta_e(rgb, t.rgb) + 1e-9);
            }
        }
    }
}

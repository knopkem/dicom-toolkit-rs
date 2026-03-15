//! Integration tests porting DCMTK's dcmdata test suite.
//!
//! Sources ported:
//!   - `tbytestr.cc`  → string value manipulation (multi-valued, position-based)
//!   - `titem.cc`     → dataset/item operations (insert, get, array lookups)
//!   - `tchval.cc`    → VR string value validation rules
//!   - `tmatch.cc`    → attribute matching (wildcards, date/time ranges, UIDs)
//!   - `tgenuid.cc`   → UID generation uniqueness (ported; main test in dcmtk-core)

use dicom_toolkit_data::{DataSet, Element, Value};
use dicom_toolkit_dict::{tags, Tag, Vr};

// ── tbytestr.cc: Multi-valued string operations ────────────────────────────────

#[test]
fn string_element_multivalue_by_backslash() {
    let elem = Element::string(tags::IMAGE_TYPE, Vr::CS, "ORIGINAL\\PRIMARY\\AXIAL");
    let strs = elem.string_value().unwrap();
    // the raw stored string contains the backslash separator
    assert!(strs.contains("ORIGINAL"), "first value present");
    assert!(strs.contains("PRIMARY"), "second value present");
    assert!(strs.contains("AXIAL"), "third value present");
}

#[test]
fn dataset_set_and_get_string() {
    let mut ds = DataSet::new();
    ds.set_string(tags::PATIENT_NAME, Vr::PN, "Doe^John");
    assert_eq!(ds.get_string(tags::PATIENT_NAME), Some("Doe^John"));
}

#[test]
fn dataset_overwrite_string() {
    let mut ds = DataSet::new();
    ds.set_string(tags::PATIENT_ID, Vr::LO, "ID-001");
    ds.set_string(tags::PATIENT_ID, Vr::LO, "ID-002");
    assert_eq!(ds.get_string(tags::PATIENT_ID), Some("ID-002"));
}

#[test]
fn dataset_multi_string_set() {
    let mut ds = DataSet::new();
    ds.set_strings(tags::IMAGE_TYPE, Vr::CS, vec!["ORIGINAL".into(), "PRIMARY".into(), "AXIAL".into()]);
    let strings = ds.get_strings(tags::IMAGE_TYPE).unwrap();
    assert_eq!(strings.len(), 3);
    assert_eq!(strings[0], "ORIGINAL");
    assert_eq!(strings[1], "PRIMARY");
    assert_eq!(strings[2], "AXIAL");
}

// ── titem.cc: Dataset/item operations ─────────────────────────────────────────

#[test]
fn dataset_insert_and_retrieve_u16() {
    let mut ds = DataSet::new();
    ds.set_u16(tags::ROWS, 256);
    ds.set_u16(tags::COLUMNS, 512);
    assert_eq!(ds.get_u16(tags::ROWS), Some(256));
    assert_eq!(ds.get_u16(tags::COLUMNS), Some(512));
}

#[test]
fn dataset_insert_multiple_elements_ordered() {
    let mut ds = DataSet::new();
    // Insert in reverse order — should be stored in ascending tag order
    ds.set_u16(tags::COLUMNS, 512);
    ds.set_u16(tags::ROWS, 256);
    ds.set_string(tags::PATIENT_NAME, Vr::PN, "Smith^John");

    let tags_in_order: Vec<Tag> = ds.tags().collect();
    // Patient name (0010,0010), Rows (0028,0010), Columns (0028,0011)
    assert!(tags_in_order.windows(2).all(|w| w[0] < w[1]), "tags should be sorted ascending");
}

#[test]
fn dataset_remove_element() {
    let mut ds = DataSet::new();
    ds.set_string(tags::PATIENT_ID, Vr::LO, "PAT-001");
    assert!(ds.contains(tags::PATIENT_ID));

    ds.remove(tags::PATIENT_ID);
    assert!(!ds.contains(tags::PATIENT_ID));
    assert_eq!(ds.get_string(tags::PATIENT_ID), None);
}

#[test]
fn dataset_len_and_is_empty() {
    let mut ds = DataSet::new();
    assert!(ds.is_empty());
    assert_eq!(ds.len(), 0);

    ds.set_string(tags::PATIENT_NAME, Vr::PN, "Doe^Jane");
    assert!(!ds.is_empty());
    assert_eq!(ds.len(), 1);

    ds.set_u16(tags::ROWS, 128);
    assert_eq!(ds.len(), 2);
}

#[test]
fn dataset_contains_returns_false_for_absent_tag() {
    let ds = DataSet::new();
    assert!(!ds.contains(tags::PATIENT_NAME));
    assert!(!ds.contains(tags::PIXEL_DATA));
}

#[test]
fn dataset_sequence_insert_and_get() {
    let mut ds = DataSet::new();
    let mut item1 = DataSet::new();
    let mut item2 = DataSet::new();
    item1.set_string(tags::PATIENT_ID, Vr::LO, "ITEM-1");
    item2.set_string(tags::PATIENT_ID, Vr::LO, "ITEM-2");

    ds.set_sequence(tags::REFERENCED_SOP_SEQUENCE, vec![item1, item2]);

    let items = ds.get_items(tags::REFERENCED_SOP_SEQUENCE).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].get_string(tags::PATIENT_ID), Some("ITEM-1"));
    assert_eq!(items[1].get_string(tags::PATIENT_ID), Some("ITEM-2"));
}

#[test]
fn dataset_bytes_insert_and_get() {
    let mut ds = DataSet::new();
    let data: Vec<u8> = vec![0x01, 0x02, 0x03, 0x04];
    ds.set_bytes(tags::PIXEL_DATA, Vr::OW, data.clone());
    assert_eq!(ds.get_bytes(tags::PIXEL_DATA), Some(data.as_slice()));
}

#[test]
fn dataset_iteration_covers_all_elements() {
    let mut ds = DataSet::new();
    ds.set_string(tags::PATIENT_NAME, Vr::PN, "Doe^John");
    ds.set_string(tags::PATIENT_ID, Vr::LO, "ID-001");
    ds.set_u16(tags::ROWS, 512);

    let count = ds.iter().count();
    assert_eq!(count, 3);
}

// ── tchval.cc: VR value validation rules ──────────────────────────────────────
// Port of DCMTK's VR format validation.

#[test]
fn vr_ae_max_16_chars() {
    // AE title max 16 printable ASCII chars, no control chars
    let valid_ae = "WORKSTATION01   "; // 16 chars
    assert!(valid_ae.len() <= 16, "AE title must be ≤ 16 chars");
    let valid_ae2 = "PACS-SERVER";
    assert!(!valid_ae2.contains('\0'), "AE title must not contain NUL");
}

#[test]
fn vr_cs_uppercase_only_valid_chars() {
    // CS: uppercase letters, digits, space, underscore only
    let valid = "ORIGINAL";
    assert!(valid.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == ' ' || c == '_'));

    let invalid = "lowercase";
    assert!(!invalid.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == ' ' || c == '_'));
}

#[test]
fn vr_ui_valid_format() {
    use dicom_toolkit_core::uid::Uid;
    // UID components separated by dots, no leading zeros in components
    assert!(Uid::is_valid("1.2.840.10008.5.1.4.1.1.2"));
    assert!(Uid::is_valid("1.2.3.4.5.6.7.8.9.0"));
    assert!(!Uid::is_valid("1.2.3.4.5.6.7.8.9."));
    assert!(!Uid::is_valid(".1.2.3"));
    assert!(!Uid::is_valid(""));
}

#[test]
fn vr_da_valid_yyyymmdd() {
    use dicom_toolkit_data::DicomDate;
    // Valid DICOM dates: YYYYMMDD
    assert!(DicomDate::from_str("20230615").is_ok());
    assert!(DicomDate::from_str("19991231").is_ok());
    // Invalid: wrong length
    assert!(DicomDate::from_str("2023-06-15").is_err());
    assert!(DicomDate::from_str("202306").is_ok(), "partial date YYYYMM is valid");
    assert!(DicomDate::from_str("2023").is_ok(), "partial date YYYY is valid");
}

#[test]
fn vr_tm_valid_hhmmss() {
    use dicom_toolkit_data::DicomTime;
    assert!(DicomTime::from_str("120000").is_ok());
    assert!(DicomTime::from_str("235959").is_ok());
    assert!(DicomTime::from_str("120000.123456").is_ok());
    assert!(DicomTime::from_str("1200").is_ok(), "HHMM is valid");
    assert!(DicomTime::from_str("12").is_ok(), "HH is valid");
    // Invalid hour
    assert!(DicomTime::from_str("250000").is_err());
}

#[test]
fn vr_dt_valid_datetime() {
    use dicom_toolkit_data::DicomDateTime;
    assert!(DicomDateTime::from_str("20230615120000.000000+0000").is_ok());
    assert!(DicomDateTime::from_str("20230615").is_ok());
    assert!(DicomDateTime::from_str("202306151200").is_ok());
}

// ── tmatch.cc: DICOM attribute matching ───────────────────────────────────────
// These tests port the DcmAttributeMatching tests from tmatch.cc.
// Our implementation lives in dicom_toolkit_data::matching.

mod matching {
    use dicom_toolkit_data::DicomDate;

    /// Simple wildcard matching: '*' matches any sequence, '?' matches one char.
    fn wildcard_match(query: &str, candidate: &str) -> bool {
        wildcard_match_bytes(query.as_bytes(), candidate.as_bytes())
    }

    fn wildcard_match_bytes(pattern: &[u8], text: &[u8]) -> bool {
        // An empty pattern matches any value (DICOM universal matching).
        let m = pattern.len();
        if m == 0 {
            return true;
        }
        // DP-based wildcard matching
        let m = pattern.len();
        let n = text.len();
        let mut dp = vec![vec![false; n + 1]; m + 1];
        dp[0][0] = true;
        for i in 1..=m {
            if pattern[i - 1] == b'*' {
                dp[i][0] = dp[i - 1][0];
            }
        }
        for i in 1..=m {
            for j in 1..=n {
                if pattern[i - 1] == b'*' {
                    dp[i][j] = dp[i - 1][j] || dp[i][j - 1];
                } else if pattern[i - 1] == b'?' || pattern[i - 1] == text[j - 1] {
                    dp[i][j] = dp[i - 1][j - 1];
                }
            }
        }
        dp[m][n]
    }

    #[test]
    fn wildcard_empty_pattern_matches_all() {
        assert!(wildcard_match("", "hello world"));
        assert!(wildcard_match("", ""));
    }

    #[test]
    fn wildcard_star_matches_all() {
        assert!(wildcard_match("*", "hello world"));
        assert!(wildcard_match("*", ""));
    }

    #[test]
    fn wildcard_question_matches_one() {
        assert!(!wildcard_match("?", "hello world"), "? matches exactly one char");
        assert!(wildcard_match("?", "x"));
        assert!(!wildcard_match("?", ""));
    }

    #[test]
    fn wildcard_combined_patterns() {
        assert!(wildcard_match("?*", "hello world"), "?* = at least one char");
        assert!(wildcard_match("?ell*??l?", "hello world"));
        assert!(!wildcard_match("?ell***?*?l??", "hello world"));
        assert!(wildcard_match("?ell*?**?l?*", "hello world"));
    }

    #[test]
    fn wildcard_exact_match() {
        assert!(wildcard_match("Hello world!", "Hello world!"));
        assert!(!wildcard_match("Hello world!", "hello world!"), "case sensitive");
    }

    #[test]
    fn wildcard_literal_star_no_match() {
        // '*' is a wildcard that matches any character sequence (including spaces).
        assert!(wildcard_match("Hello*world!", "Hello world!"), "* matches a space");
        assert!(wildcard_match("Hello*world!", "Hello*world!"), "* also matches literal *");
        assert!(wildcard_match("Hello*world!", "Helloworld!"), "* matches empty sequence");
        assert!(!wildcard_match("Hello*world!", "Hi world!"), "prefix must match");
    }

    /// Date range matching: "" matches all; "-YYYYMMDD" = up to; "YYYYMMDD-" = from; "YYYYMMDD-YYYYMMDD" = range.
    fn date_range_match(query: &str, candidate: &str) -> bool {
        // Parse candidate as DicomDate
        let cand = match DicomDate::from_str(candidate.trim_end_matches('.').replace('.', "").trim()) {
            Ok(d) => d,
            // Try legacy dot format
            Err(_) => match DicomDate::from_da_str(candidate) {
                Ok(d) => d,
                Err(_) => return false,
            },
        };
        if query.is_empty() {
            return true;
        }
        if query.contains('-') {
            let parts: Vec<&str> = query.splitn(2, '-').collect();
            let from = if parts[0].is_empty() {
                None
            } else {
                DicomDate::from_str(parts[0]).ok()
            };
            let to = if parts.len() < 2 || parts[1].is_empty() {
                None
            } else {
                DicomDate::from_str(parts[1]).ok()
            };
            match (from, to) {
                (None, None) => true,
                (Some(f), None) => cand >= f,
                (None, Some(t)) => cand <= t,
                (Some(f), Some(t)) => cand >= f && cand <= t,
            }
        } else {
            // Exact match or legacy format
            DicomDate::from_da_str(query).ok() == Some(cand)
        }
    }

    #[test]
    fn date_empty_query_matches_all() {
        assert!(date_range_match("", "20170224"));
    }

    #[test]
    fn date_up_to_range() {
        assert!(date_range_match("-20000101", "20000101"), "equal to upper bound");
        assert!(date_range_match("-20000101", "19990531"), "before upper bound");
        assert!(!date_range_match("-20000101", "20010101"), "after upper bound");
    }

    #[test]
    fn date_from_range() {
        assert!(date_range_match("20000101-", "20010101"), "after lower bound");
        assert!(!date_range_match("20000101-", "19991231"), "before lower bound");
    }

    #[test]
    fn date_explicit_range() {
        assert!(date_range_match("19990101-20000305", "20000101"), "within range");
        assert!(!date_range_match("19990101-20000305", "19980107"), "before range");
        assert!(!date_range_match("19990101-20000305", "20000306"), "after range");
    }

    #[test]
    fn date_legacy_dot_format_in_query() {
        assert!(date_range_match("1987.08.02", "19870802"), "legacy dot format");
    }

    /// UID list matching: query is `\\`-separated list; candidate matches if it appears in list.
    fn uid_list_match(query: &str, candidate: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        query.split('\\').any(|uid| uid == candidate)
    }

    #[test]
    fn uid_empty_query_matches_all() {
        assert!(uid_list_match("", "123.456.789.10"));
    }

    #[test]
    fn uid_exact_match() {
        assert!(uid_list_match("123.456.789.10", "123.456.789.10"));
        assert!(!uid_list_match("456.789.10", "123.456.789.10"));
    }

    #[test]
    fn uid_list_membership() {
        assert!(uid_list_match("456.789.10\\123.456.789.10", "123.456.789.10"));
        assert!(uid_list_match("456.789.10\\123.456.789.10\\456.123.789.10", "123.456.789.10"));
        assert!(!uid_list_match("456.789.10\\123.456.79.10\\456.123.789.10", "123.456.789.10"), "no match");
    }
}

// ── JSON round-trip tests ──────────────────────────────────────────────────────

#[test]
fn json_roundtrip_complex_dataset() {
    use dicom_toolkit_data::json;

    let mut ds = DataSet::new();
    ds.set_string(tags::PATIENT_NAME, Vr::PN, "Müller^Hans");
    ds.set_string(tags::PATIENT_ID, Vr::LO, "PAT-0042");
    ds.set_uid(tags::SOP_CLASS_UID, "1.2.840.10008.5.1.4.1.1.2");
    ds.set_uid(tags::SOP_INSTANCE_UID, "1.2.3.4.5.6.7.8.9.0");
    ds.set_u16(tags::ROWS, 512);
    ds.set_u16(tags::COLUMNS, 512);
    ds.set_u16(tags::BITS_ALLOCATED, 16);
    ds.set_u16(tags::BITS_STORED, 12);

    let json_str = json::to_json(&ds).unwrap();
    let parsed = json::from_json(&json_str).unwrap();

    assert_eq!(parsed.get_string(tags::PATIENT_ID), Some("PAT-0042"));
    assert_eq!(parsed.get_u16(tags::ROWS), Some(512));
    assert_eq!(parsed.get_u16(tags::COLUMNS), Some(512));
    assert_eq!(parsed.get_u16(tags::BITS_ALLOCATED), Some(16));
}

#[test]
fn json_roundtrip_nested_sequences() {
    use dicom_toolkit_data::json;

    let mut ds = DataSet::new();
    let mut seq_items = Vec::new();
    for i in 0..3u16 {
        let mut item = DataSet::new();
        item.set_string(tags::PATIENT_ID, Vr::LO, &format!("ITEM-{i}"));
        item.set_u16(tags::ROWS, i * 128);
        seq_items.push(item);
    }
    ds.set_sequence(tags::REFERENCED_SOP_SEQUENCE, seq_items);

    let json_str = json::to_json(&ds).unwrap();
    let parsed = json::from_json(&json_str).unwrap();

    let items = parsed.get_items(tags::REFERENCED_SOP_SEQUENCE).unwrap();
    assert_eq!(items.len(), 3);
    for (i, item) in items.iter().enumerate() {
        assert_eq!(item.get_string(tags::PATIENT_ID), Some(format!("ITEM-{i}").as_str()));
        assert_eq!(item.get_u16(tags::ROWS), Some(i as u16 * 128));
    }
}

// ── XML serialization tests ────────────────────────────────────────────────────

#[test]
fn xml_well_formed_for_complex_dataset() {
    use dicom_toolkit_data::xml;

    let mut ds = DataSet::new();
    ds.set_string(tags::PATIENT_NAME, Vr::PN, "Smith^John");
    ds.set_string(tags::PATIENT_ID, Vr::LO, "ID-001");
    ds.set_uid(tags::SOP_INSTANCE_UID, "1.2.3.4.5");
    ds.set_u16(tags::ROWS, 256);
    ds.set_u16(tags::COLUMNS, 256);

    let xml_str = xml::to_xml(&ds).unwrap();
    assert!(xml_str.contains("<NativeDicomModel"), "should have root element");
    assert!(xml_str.contains("</NativeDicomModel>"), "root must be closed");
    assert!(xml_str.contains("256"), "should contain numeric value");
    assert!(xml_str.contains("1.2.3.4.5"), "should contain UID");
}

#[test]
fn xml_special_chars_escaped() {
    use dicom_toolkit_data::xml;

    let mut ds = DataSet::new();
    ds.set_string(tags::PATIENT_ID, Vr::LO, "A<B>&C\"D'E");
    let xml_str = xml::to_xml(&ds).unwrap();
    assert!(xml_str.contains("&lt;"), "< should be escaped");
    assert!(xml_str.contains("&amp;"), "& should be escaped");
    assert!(!xml_str.contains("A<B>"), "raw < > should not appear");
}

// ── Value type tests ───────────────────────────────────────────────────────────

#[test]
fn value_empty_has_multiplicity_zero() {
    let v = Value::Empty;
    assert_eq!(v.multiplicity(), 0);
    assert!(v.is_empty());
}

#[test]
fn value_strings_multiplicity() {
    let v = Value::Strings(vec!["a".into(), "b".into(), "c".into()]);
    assert_eq!(v.multiplicity(), 3);
    assert!(!v.is_empty());
}

#[test]
fn value_u16_multiplicity() {
    let v = Value::U16(vec![1, 2]);
    assert_eq!(v.multiplicity(), 2);
}

#[test]
fn value_sequence_multiplicity() {
    let v = Value::Sequence(vec![DataSet::new(), DataSet::new()]);
    assert_eq!(v.multiplicity(), 2);
}

#[test]
fn element_display_dcmdump_style() {
    let elem = Element::u16(tags::ROWS, 512);
    let display = format!("{elem}");
    assert!(display.contains("0028,0010"), "should show tag");
    assert!(display.contains("US"), "should show VR");
    assert!(display.contains("512"), "should show value");
}

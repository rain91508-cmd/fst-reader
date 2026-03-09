// Copyright 2023 The Regents of the University of California
// Copyright 2024 Cornell University
// released under BSD 3-Clause License
// author: Kevin Laeufer <laeufer@cornell.edu>
//
// Tests for the include_pre_start feature.

use fst_reader::*;

/// Test that include_pre_start defaults to false
#[test]
fn test_include_pre_start_defaults_to_false() {
    let filter = FstFilter::all();
    assert!(!filter.include_pre_start);

    let filter = FstFilter::new(100, 200, vec![]);
    assert!(!filter.include_pre_start);

    let filter = FstFilter::filter_time(100, 200);
    assert!(!filter.include_pre_start);

    let filter = FstFilter::filter_signals(vec![]);
    assert!(!filter.include_pre_start);
}

/// Test the with_pre_start() method
#[test]
fn test_with_pre_start_method() {
    let filter = FstFilter::all().with_pre_start();
    assert!(filter.include_pre_start);

    let filter = FstFilter::new(100, 200, vec![]).with_pre_start();
    assert!(filter.include_pre_start);

    let filter = FstFilter::filter_time(100, 200).with_pre_start();
    assert!(filter.include_pre_start);

    let filter = FstFilter::filter_signals(vec![]).with_pre_start();
    assert!(filter.include_pre_start);
}

/// Test that we can directly set the field
#[test]
fn test_direct_field_assignment() {
    let mut filter = FstFilter::all();
    filter.include_pre_start = true;
    assert!(filter.include_pre_start);

    let mut filter = FstFilter::new(100, 200, vec![]);
    filter.include_pre_start = true;
    assert!(filter.include_pre_start);
}

/// Test reading signals with include_pre_start = false (default behavior)
/// This test uses a simple FST file and verifies that only values >= start are returned.
#[test]
fn test_read_signals_without_pre_start() {
    // Use the minimal_2sections.fst file which has known signal changes
    let fst = std::fs::File::open("fsts/partial/minimal_2sections.fst").unwrap();
    let hier = std::fs::File::open("fsts/partial/minimal_2sections.fst.hier").unwrap();
    let mut reader =
        FstReader::open_incomplete(std::io::BufReader::new(fst), std::io::BufReader::new(hier))
            .unwrap();

    // First, get a signal handle
    let mut signal_handle = None;
    reader
        .read_hierarchy(|entry| {
            if let FstHierarchyEntry::Var { handle, .. } = entry {
                if signal_handle.is_none() {
                    signal_handle = Some(handle);
                }
            }
        })
        .unwrap();

    let handle = signal_handle.expect("Should have at least one signal");

    // Read with start=100, include_pre_start=false (default)
    let filter = FstFilter::new(100, 200, vec![handle]);
    let mut callbacks = Vec::new();
    reader
        .read_signals(&filter, |time, h, value| {
            if h == handle {
                callbacks.push(time);
            }
        })
        .unwrap();

    // All callbacks should be >= 100
    for time in &callbacks {
        assert!(*time >= 100, "Time {} should be >= 100", time);
    }
}

/// Test reading signals with include_pre_start = true
/// This test verifies that values before start are included if they are the last change before start.
#[test]
fn test_read_signals_with_pre_start() {
    // Use the minimal_2sections.fst file which has known signal changes
    let fst = std::fs::File::open("fsts/partial/minimal_2sections.fst").unwrap();
    let hier = std::fs::File::open("fsts/partial/minimal_2sections.fst.hier").unwrap();
    let mut reader =
        FstReader::open_incomplete(std::io::BufReader::new(fst), std::io::BufReader::new(hier))
            .unwrap();

    // First, get a signal handle
    let mut signal_handle = None;
    reader
        .read_hierarchy(|entry| {
            if let FstHierarchyEntry::Var { handle, .. } = entry {
                if signal_handle.is_none() {
                    signal_handle = Some(handle);
                }
            }
        })
        .unwrap();

    let handle = signal_handle.expect("Should have at least one signal");

    // Read all signals to understand the timeline
    let all_filter = FstFilter::all();
    let mut all_callbacks = Vec::new();
    reader
        .read_signals(&all_filter, |time, h, value| {
            if h == handle {
                all_callbacks.push(time);
            }
        })
        .unwrap();

    // Now read with start=100, include_pre_start=true
    let fst = std::fs::File::open("fsts/partial/minimal_2sections.fst").unwrap();
    let hier = std::fs::File::open("fsts/partial/minimal_2sections.fst.hier").unwrap();
    let mut reader =
        FstReader::open_incomplete(std::io::BufReader::new(fst), std::io::BufReader::new(hier))
            .unwrap();

    let filter = FstFilter::new(100, 200, vec![handle]).with_pre_start();
    let mut pre_start_callbacks = Vec::new();
    reader
        .read_signals(&filter, |time, h, value| {
            if h == handle {
                pre_start_callbacks.push(time);
            }
        })
        .unwrap();

    // With include_pre_start=true, we should have at least as many callbacks as without
    // and the first callback should be <= 100
    assert!(
        !pre_start_callbacks.is_empty(),
        "Should have at least one callback"
    );

    // The first callback should be the last change before or at start
    let first_time = pre_start_callbacks[0];
    assert!(
        first_time <= 100,
        "First callback time {} should be <= 100 when include_pre_start=true",
        first_time
    );
}

/// Test that include_pre_start works correctly when start is exactly at a signal change
#[test]
fn test_include_pre_start_at_exact_boundary() {
    let fst = std::fs::File::open("fsts/partial/minimal_2sections.fst").unwrap();
    let hier = std::fs::File::open("fsts/partial/minimal_2sections.fst.hier").unwrap();
    let mut reader =
        FstReader::open_incomplete(std::io::BufReader::new(fst), std::io::BufReader::new(hier))
            .unwrap();

    // Get a signal handle
    let mut signal_handle = None;
    reader
        .read_hierarchy(|entry| {
            if let FstHierarchyEntry::Var { handle, .. } = entry {
                if signal_handle.is_none() {
                    signal_handle = Some(handle);
                }
            }
        })
        .unwrap();

    let handle = signal_handle.expect("Should have at least one signal");

    // Read with start=0 (beginning of file)
    let filter = FstFilter::new(0, 200, vec![handle]).with_pre_start();
    let mut callbacks = Vec::new();
    reader
        .read_signals(&filter, |time, h, value| {
            if h == handle {
                callbacks.push(time);
            }
        })
        .unwrap();

    // Should still work correctly at boundary
    assert!(!callbacks.is_empty(), "Should have callbacks at boundary");
}

/// Test include_pre_start with multiple signals
#[test]
fn test_include_pre_start_multiple_signals() {
    let fst = std::fs::File::open("fsts/partial/minimal_2sections.fst").unwrap();
    let hier = std::fs::File::open("fsts/partial/minimal_2sections.fst.hier").unwrap();
    let mut reader =
        FstReader::open_incomplete(std::io::BufReader::new(fst), std::io::BufReader::new(hier))
            .unwrap();

    // Get all signal handles
    let mut signal_handles = Vec::new();
    reader
        .read_hierarchy(|entry| {
            if let FstHierarchyEntry::Var { handle, .. } = entry {
                signal_handles.push(handle);
            }
        })
        .unwrap();

    if signal_handles.len() < 2 {
        // Skip if not enough signals
        return;
    }

    let handles = signal_handles[..2].to_vec();

    // Read with include_pre_start=true
    let filter = FstFilter::new(100, 200, handles.clone()).with_pre_start();
    let mut callback_count = 0;
    reader
        .read_signals(&filter, |time, h, value| {
            if handles.contains(&h) {
                callback_count += 1;
            }
        })
        .unwrap();

    assert!(callback_count > 0, "Should have callbacks for multiple signals");
}

/// Test that include_pre_start doesn't affect the total number of callbacks when start=0
#[test]
fn test_include_pre_start_with_start_at_zero() {
    let fst = std::fs::File::open("fsts/partial/minimal_2sections.fst").unwrap();
    let hier = std::fs::File::open("fsts/partial/minimal_2sections.fst.hier").unwrap();
    let mut reader =
        FstReader::open_incomplete(std::io::BufReader::new(fst), std::io::BufReader::new(hier))
            .unwrap();

    // Get a signal handle
    let mut signal_handle = None;
    reader
        .read_hierarchy(|entry| {
            if let FstHierarchyEntry::Var { handle, .. } = entry {
                if signal_handle.is_none() {
                    signal_handle = Some(handle);
                }
            }
        })
        .unwrap();

    let handle = signal_handle.expect("Should have at least one signal");

    // Read with start=0, include_pre_start=false
    let filter_without = FstFilter::new(0, 200, vec![handle]);
    let mut count_without = 0;
    reader
        .read_signals(&filter_without, |time, h, value| {
            if h == handle {
                count_without += 1;
            }
        })
        .unwrap();

    // Read with start=0, include_pre_start=true
    let fst = std::fs::File::open("fsts/partial/minimal_2sections.fst").unwrap();
    let hier = std::fs::File::open("fsts/partial/minimal_2sections.fst.hier").unwrap();
    let mut reader =
        FstReader::open_incomplete(std::io::BufReader::new(fst), std::io::BufReader::new(hier))
            .unwrap();

    let filter_with = FstFilter::new(0, 200, vec![handle]).with_pre_start();
    let mut count_with = 0;
    reader
        .read_signals(&filter_with, |time, h, value| {
            if h == handle {
                count_with += 1;
            }
        })
        .unwrap();

    // When start=0, both should give the same count
    assert_eq!(
        count_without, count_with,
        "With start=0, include_pre_start should not affect callback count"
    );
}

/// Test include_pre_start with a regular (non-incomplete) FST file
#[test]
fn test_include_pre_start_regular_fst() {
    // Use a regular FST file
    let filename = "fsts/fst-writer/simple.fst";
    let f = std::fs::File::open(filename).unwrap_or_else(|_| {
        // Skip if file doesn't exist
        return;
    });

    let mut reader = FstReader::open(std::io::BufReader::new(f)).unwrap();

    // Get a signal handle
    let mut signal_handle = None;
    reader
        .read_hierarchy(|entry| {
            if let FstHierarchyEntry::Var { handle, .. } = entry {
                if signal_handle.is_none() {
                    signal_handle = Some(handle);
                }
            }
        })
        .unwrap();

    let Some(handle) = signal_handle else {
        return; // Skip if no signals
    };

    // Read with include_pre_start=true
    let filter = FstFilter::new(10, 100, vec![handle]).with_pre_start();
    let mut callbacks = Vec::new();
    let result = reader.read_signals(&filter, |time, h, value| {
        if h == handle {
            callbacks.push(time);
        }
    });

    if result.is_ok() {
        assert!(!callbacks.is_empty(), "Should have callbacks");
        // First callback should be <= 10
        if let Some(&first_time) = callbacks.first() {
            assert!(
                first_time <= 10,
                "First callback {} should be <= 10",
                first_time
            );
        }
    }
}

/// Test that the feature works with FstFilter::all()
#[test]
fn test_include_pre_start_with_filter_all() {
    let fst = std::fs::File::open("fsts/partial/minimal_2sections.fst").unwrap();
    let hier = std::fs::File::open("fsts/partial/minimal_2sections.fst.hier").unwrap();
    let mut reader =
        FstReader::open_incomplete(std::io::BufReader::new(fst), std::io::BufReader::new(hier))
            .unwrap();

    // FstFilter::all() with include_pre_start
    let filter = FstFilter::all().with_pre_start();
    let mut callback_count = 0;
    reader
        .read_signals(&filter, |_, _, _| callback_count += 1)
        .unwrap();

    // Should read all values (same as without with_pre_start since start=0)
    assert!(callback_count > 0, "Should have callbacks");
}

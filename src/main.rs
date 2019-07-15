#![warn(clippy::all)]
use std::fs;
use std::path::PathBuf;

use clap::arg_enum;
use snafu::{ResultExt, Snafu};
use structopt::StructOpt;

mod patch;

use patch::{Patch, PatchSection};

arg_enum! {
    #[derive(Debug)]
    enum Mode {
        Diff,
        Patch
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "Patcher", about = "Smart patcher")]
struct Options {
    #[structopt(
        short,
        long,
        raw(possible_values = "&Mode::variants()"),
        case_insensitive = true,
        default_value = "Diff"
    )]
    mode: Mode,
    #[structopt(
        short,
        long,
        default_value = "6",
        help = "Allow n bytes to be included if they are just outliers."
    )]
    follow: usize,
    #[structopt(short, long, help = "Only patch ASCII bytes in the range 0x30-0x71")]
    only_char: bool,
    #[structopt(short, long, help = "Detect if section has appears multiple times.")]
    detect: bool,
    #[structopt(short, long, parse(from_os_str))]
    test: Option<PathBuf>,
    #[structopt(index = 1, required = true, name = "FILE1", parse(from_os_str))]
    input: PathBuf,
    #[structopt(index = 2, required = true, name = "FILE2", parse(from_os_str))]
    patch: PathBuf,
    #[structopt(index = 3, name = "FILE3", parse(from_os_str))]
    output: Option<PathBuf>,
}

#[derive(Debug, Snafu)]
enum Error {
    #[snafu(display("Unable to read source file {}: {}", path.display(), source))]
    ReadSource {
        source: std::io::Error,
        path: PathBuf,
    },
    #[snafu(display("Unable to read transformed file {}: {}", path.display(), source))]
    ReadTarget {
        source: std::io::Error,
        path: PathBuf,
    },
    #[snafu(display("Unable to read test file for growing search data {}: {}", path.display(), source))]
    ReadTest {
        source: std::io::Error,
        path: PathBuf,
    },
    #[snafu(display(
        "Source and transformed file have different sizes {}!={}: {}",
        source_size,
        target_size,
        source
    ))]
    SizeMismatch {
        source: std::io::Error,
        source_size: u64,
        target_size: u64,
    },
    #[snafu(display("Unable to read patch file {}: {}", path.display(), source))]
    ReadPatch {
        source: std::io::Error,
        path: PathBuf,
    },
    #[snafu(display("Error converting path to json {:?}: {}", patch, source))]
    SerializePatch {
        source: serde_json::error::Error,
        patch: Patch,
    },
    #[snafu(display("Unable to write patch file {}: {}", path.display(), source))]
    WritePatch {
        source: std::io::Error,
        path: PathBuf,
    },
    #[snafu(display("Unable to write patched file {}: {}", path.display(), source))]
    WritePatchedFile {
        source: std::io::Error,
        path: PathBuf,
    },
}

type Result<T, E = Error> = std::result::Result<T, E>;

fn main() -> Result<()> {
    let opt = Options::from_args();
    match opt.mode {
        Mode::Diff => {
            build_patch(&opt)?;
        }
        Mode::Patch => {
            apply_patch(&opt)?;
        }
    }
    Ok(())
}

fn build_patch(opt: &Options) -> Result<()> {
    let input_size = fs::metadata(&opt.input).context(ReadSource { path: &opt.input })?;
    let patched_size = fs::metadata(&opt.patch).context(ReadTarget { path: &opt.patch })?;

    if input_size.len() != patched_size.len() {
        println!("Different file sizes.");
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "File size mismatch.",
        ))
        .context(SizeMismatch {
            source_size: input_size.len(),
            target_size: patched_size.len(),
        });
    }

    let input = fs::read(&opt.input).context(ReadSource { path: &opt.input })?;
    let patched = fs::read(&opt.patch).context(ReadTarget { path: &opt.patch })?;

    let mut patch = Patch::default();
    let mut patching = false;
    let mut section_index = 0;
    let mut fail_count = 0;
    let mut fail_continue = false;
    let mut extra_search: Vec<u8> = Vec::new();
    let mut extra_data: Vec<u8> = Vec::new();

    println!("Scanning files for differences...");

    for i in 0..input.len() {
        let valid = input[i] >= 0x30 && input[i] <= 0x71 || !opt.only_char;
        if input[i] != patched[i] && valid {
            if !patching && !fail_continue {
                patching = true;
                patch.sections.push(PatchSection::default());
                section_index += 1;
                patch.sections[section_index - 1].id = section_index as u32;
                patch.sections[section_index - 1].start = i;
                fail_count = 0;
            }

            if fail_continue {
                patch.sections[section_index - 1]
                    .search
                    .append(&mut extra_search);
                patch.sections[section_index - 1]
                    .data
                    .append(&mut extra_data);
                extra_search.clear();
                extra_data.clear();
                patching = true;
            }

            patch.sections[section_index - 1].search.push(input[i]);
            patch.sections[section_index - 1].data.push(patched[i]);
            patch.sections[section_index - 1].end = i;
            fail_continue = false;
        } else {
            if fail_count < opt.follow && section_index > 0 && valid {
                extra_search.push(input[i]);
                extra_data.push(patched[i]);
                fail_continue = true;
            } else {
                extra_search.clear();
                extra_data.clear();
                fail_continue = false;
            }
            fail_count += 1;
            patching = false;
        }
    }

    println!("Fixing small sections...");
    if !patch.sections.is_empty() {
        for i in 0..patch.sections.len() {
            let mut section = &mut patch.sections[i];
            grow_section(&mut section, &input, &patched, opt)?;
        }
    }

    println!("Merging sections...");
    section_merge(&mut patch);

    println!("Final patch has {} sections.", &patch.sections.len());
    let mut patch_filename = match &opt.output {
        Some(x) => x.clone(),
        None => opt.input.clone(),
    };
    patch_filename.set_extension("rbp");

    let coded = bincode::serialize(&patch).unwrap();
    fs::write(&patch_filename, coded).context(WritePatch {
        path: &patch_filename,
    })?;

    let coded = serde_json::to_string(&patch).context(SerializePatch { patch })?;
    patch_filename.set_extension("json");
    fs::write(&patch_filename, coded).context(WritePatch {
        path: &patch_filename,
    })?;
    Ok(())
}

/// Grow sections if they appear many times on the base file.
fn grow_section(
    section: &mut PatchSection,
    input: &[u8],
    patched: &[u8],
    opt: &Options,
) -> Result<()> {
    let mut new_section = section.clone();
    let max_grow = 10;
    let mut after = 0;
    let mut section_done = false;
    let test_file = match &opt.test {
        Some(x) => fs::read(x).context(ReadTest { path: x })?,
        None => input.to_vec(),
    };
    while after < max_grow && !section_done {
        let mut i = 0;
        let mut section_count = 0;
        while i < test_file.len() {
            if section_count > 1 {
                break;
            }

            if test_file[i] == new_section.search[0] {
                let mut valid_section = true;
                // Validate section
                for j in 0..new_section.search.len() {
                    if i + j >= test_file.len() || test_file[i + j] != new_section.search[j] {
                        valid_section = false;
                        break;
                    }
                }
                if valid_section {
                    section_count += 1;
                    i += new_section.search.len();
                    continue;
                }
            }
            i += 1;
        }
        if section_count > 1 {
            // println!("Detected more than one Section {:02}. Adding one extra byte.", new_section.id);
            after += 1;
            section_append(&mut new_section, input, patched, 1);
        } else {
            section_done = true;
        }
    }
    if section_done && after > 0 {
        println!("Fixed Section {:02}", new_section.id);
        println!(
            "Old Section {} search pattern: {:02X?}",
            section.id, &section.search
        );
        section.start = new_section.start;
        section.end = new_section.end;
        section.search.clear();
        section.data.clear();
        section.search.append(&mut new_section.search);
        section.data.append(&mut new_section.data);
        println!(
            "New Section {} search pattern: {:02X?}",
            section.id, &section.search
        );
    } else if after > 0 {
        println!("Fixed Section {:02}", new_section.id);
        println!(
            "Old Section {} search pattern: {:02X?}",
            section.id, section.search
        );
    }
    Ok(())
}

/// Append an extra byte from the source files
fn section_append(section: &mut PatchSection, input: &[u8], patched: &[u8], amount: usize) {
    let mut after_search = input
        [(section.end + 1)..=(std::cmp::min(section.end + amount, input.len()))]
        .to_vec()
        .clone();
    let mut after_data = patched
        [(section.end + 1)..=(std::cmp::min(section.end + amount, input.len()))]
        .to_vec()
        .clone();
    section.search.append(&mut after_search);
    section.data.append(&mut after_data);
    section.end += amount;
}

/// Merge sections that overlap with a lazy strategy
fn section_merge(patch: &mut Patch) -> bool {
    if patch.sections.len() == 1 {
        return true;
    }
    let mut new_patch = patch.clone();
    let mut i = 0;
    let mut count = new_patch.sections.len() - 1;
    while i < count {
        let s1 = &new_patch.sections[i].end;
        let s2 = &new_patch.sections[i + 1].start;
        if s1 >= s2 {
            let start = new_patch.sections[i].end - new_patch.sections[i].start;
            let end = new_patch.sections[i + 1].end - new_patch.sections[i].end;
            let mut new_search = new_patch.sections[i + 1].search[start..=end]
                .to_vec()
                .clone();
            new_patch.sections[i].search.append(&mut new_search);
            new_patch.sections[i].end = new_patch.sections[i + 1].end;

            let mut new_data = new_patch.sections[i + 1].data[start..=end].to_vec().clone();
            new_patch.sections[i].data.append(&mut new_data);
            new_patch.sections.remove(i + 1);
            count = new_patch.sections.len() - 1;
        } else {
            i += 1;
        }
    }
    if new_patch.sections.len() < patch.sections.len() {
        println!(
            "Merged {} sections.",
            patch.sections.len() - new_patch.sections.len()
        );
        patch.sections.clear();
        patch.sections.append(&mut new_patch.sections);
    }
    true
}

/// Applies a patch file
fn apply_patch(opt: &Options) -> Result<()> {
    let input = fs::read(&opt.input).context(ReadSource { path: &opt.input })?;
    let path = std::path::Path::new(&opt.patch);
    let patched = fs::read(&opt.patch).context(ReadPatch { path: &opt.patch })?;

    // Loads our patch information (can be bincode or json)
    let patch: Patch = if path.extension().unwrap_or_default() == "json" {
        serde_json::from_str(&String::from_utf8(patched).unwrap()).unwrap()
    } else {
        bincode::deserialize(&patched).unwrap()
    };
    println!("Sections found: {}", &patch.sections.len());
    let mut section_index = 0;
    let mut i;
    let mut result: Vec<u8> = Vec::new();
    let mut section_count = 0;
    if opt.detect {
        for (k, section) in patch.sections.iter().enumerate() {
            i = 0;
            while i < input.len() {
                if input[i] == section.search[0] {
                    let mut valid_section = true;
                    // Validate section
                    for j in 0..section.search.len() {
                        if input[i + j] != section.search[j] {
                            valid_section = false;
                            break;
                        }
                    }
                    if valid_section {
                        section_count += 1;
                        println!("Detected section {:2} at offset {}", k + 1, i);
                        i += section.search.len();
                        continue;
                    }
                }
                i += 1;
            }
        }
    }
    if section_count > patch.sections.len() {
        panic!("Too many sections found.");
    }

    i = 0;
    // Search the input file for the patch sections
    while i < input.len() && section_index < patch.sections.len() {
        let section = &patch.sections[section_index];

        if input[i] == section.search[0] {
            let mut valid_section = true;
            // Validate section
            for j in 0..section.search.len() {
                if input[i + j] != section.search[j] {
                    valid_section = false;
                    break;
                }
            }

            // Apply the section
            if valid_section {
                println!(
                    "Applied section {:02} at index {} with len {}",
                    section_index + 1,
                    i,
                    section.data.len()
                );
                result.append(&mut section.data.clone());
                section_index += 1;
                i += section.search.len();
                continue;
            }
        }
        result.push(input[i]);
        i += 1;
    }

    // Add any missing file bytes.
    if i < input.len() {
        let mut section = input[i..input.len()].to_vec().clone();
        result.append(&mut section);
    }

    // Check if we parsed all sections
    if section_index != patch.sections.len() {
        println!("Failed to apply patch.");
    } else {
        // And save the patched file.
        println!("Patch applied.");
        let mut patch_filename = opt.input.clone();
        patch_filename.set_extension("patched");
        fs::write(&patch_filename, &result).context(WritePatchedFile {
            path: &patch_filename,
        })?;
    }
    Ok(())
}

use clap::arg_enum;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::prelude::*;
use std::path::PathBuf;
use structopt::StructOpt;

arg_enum! {
    #[derive(Debug)]
    enum Mode {
        Diff,
        Patch
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "basic")]
struct Options {
    #[structopt(
        short,
        long,
        raw(possible_values = "&Mode::variants()"),
        case_insensitive = true,
        default_value = "Diff"
    )]
    mode: Mode,
    #[structopt(short, long, default_value = "1")]
    bad_bytes: usize,
    #[structopt(short, long)]
    only_char: bool,
    #[structopt(index = 1, required = true, name = "FILE1", parse(from_os_str))]
    input: PathBuf,
    #[structopt(index = 2, required = true, name = "FILE2", parse(from_os_str))]
    patch: PathBuf,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Default)]
struct Patch {
    sections: Vec<PatchSection>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Default)]
struct PatchSection {
    start: usize,
    end: usize,
    #[serde(with = "serde_bytes")]
    search: Vec<u8>,
    data: Vec<u8>,
}

fn main() -> std::io::Result<()> {
    let opt = Options::from_args();
    println!("{:?}", opt);
    match opt.mode {
        Mode::Diff => {
            let input_size = fs::metadata(&opt.input)?;
            let patched_size = fs::metadata(&opt.patch)?;

            if input_size.len() != patched_size.len() {
                panic!("Different file sizes.");
            }

            let input = fs::read(&opt.input)?;
            let patched = fs::read(&opt.patch)?;

            let mut patch = Patch::default();
            let mut patching = false;
            let mut section_index = 0;
            let mut fail_count = 0;
            let mut fail_continue = false;
            let mut failedo: Vec<u8> = Vec::new();
            let mut failedp: Vec<u8> = Vec::new();

            for i in 0..input.len() {
                let valid = !opt.only_char || (opt.only_char && input[i] >= 0x30 && input[i] <= 0x71);
                if input[i] != patched[i] {
                    if !opt.only_char || (opt.only_char && input[i] >= 0x30 && input[i] <= 0x71) {
                        if !patching && !fail_continue {
                            if section_index > 0 {
                                println!(
                                    "Section {}: {:02X?}",
                                    section_index - 1,
                                    &patch.sections[section_index - 1].search
                                );
                            }
                            println!("Starting section: {}", section_index);
                            patching = true;
                            patch.sections.push(PatchSection::default());
                            section_index += 1;
                            patch.sections[section_index - 1].start = i;
                            fail_count = 0;
                        }
                        if fail_continue {
                            println!("Adding {} bytes missed equal in range.", failedo.len());
                            patch.sections[section_index - 1]
                                .search
                                .append(&mut failedo);
                            patch.sections[section_index - 1].data.append(&mut failedp);
                            failedo.clear();
                            failedp.clear();
                            patching = true;
                        }

                        patch.sections[section_index - 1].search.push(input[i]);
                        patch.sections[section_index - 1].data.push(patched[i]);
                        patch.sections[section_index - 1].end = i;
                    } else {
                        patching = false;
                    }
                    fail_continue = false;
                } else {
                    if fail_count < opt.bad_bytes && section_index > 0 {
                        if !opt.only_char || (opt.only_char && input[i] >= 0x30 && input[i] <= 0x71)
                        {
                            failedo.push(input[i]);
                            failedp.push(patched[i]);
                            fail_continue = true;
                        } else {
                            failedo.clear();
                            failedp.clear();
                            fail_continue = false;
                        }
                    } else {
                        failedo.clear();
                        failedp.clear();
                        fail_continue = false;
                    }
                    fail_count += 1;
                    patching = false;
                }
            }
            println!("Sections found: {}", &patch.sections.len());
            let mut patch_filename = opt.input.clone();
            patch_filename.set_extension("rbp");

            let coded = bincode::serialize(&patch).unwrap();
            fs::write(&patch_filename, coded)?;

            let coded = serde_json::to_string(&patch)?;
            patch_filename.set_extension("json");
            fs::write(&patch_filename, coded)?;
        }
        Mode::Patch => {}
    }
    Ok(())
}

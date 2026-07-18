use crate::{EnvironmentPolicy, ProcessSpec};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStrExt;

/// Builds the mutable CreateProcess command line using the Windows CRT quoting contract.
pub(super) fn build_command_line(spec: &ProcessSpec) -> io::Result<Vec<u16>> {
    let mut command_line = OsString::new();
    append_quoted_argument(&mut command_line, spec.program())?;
    for argument in spec.args_iter() {
        command_line.push(" ");
        append_quoted_argument(&mut command_line, argument)?;
    }
    wide_nul(&command_line, "command line")
}

/// Quotes one argv element so backslashes before quotes and the closing quote remain lossless.
fn append_quoted_argument(command_line: &mut OsString, argument: &OsStr) -> io::Result<()> {
    let text = argument.to_string_lossy();
    if text.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "command argument contains NUL",
        ));
    }
    let requires_quotes = text.is_empty() || text.chars().any(char::is_whitespace);
    if !requires_quotes && !text.contains('"') {
        command_line.push(argument);
        return Ok(());
    }

    command_line.push("\"");
    let mut backslashes = 0_usize;
    for character in text.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        if character == '"' {
            command_line.push("\\".repeat(backslashes * 2 + 1));
            command_line.push("\"");
        } else {
            command_line.push("\\".repeat(backslashes));
            command_line.push(character.to_string());
        }
        backslashes = 0;
    }
    command_line.push("\\".repeat(backslashes * 2));
    command_line.push("\"");
    Ok(())
}

/// Produces the case-insensitively sorted double-NUL-terminated Unicode environment block.
pub(super) fn build_environment_block(spec: &ProcessSpec) -> io::Result<Vec<u16>> {
    let mut variables = BTreeMap::<String, (OsString, OsString)>::new();
    if spec.environment_policy() == EnvironmentPolicy::InheritAndOverride {
        for (key, value) in std::env::vars_os() {
            variables.insert(key.to_string_lossy().to_uppercase(), (key, value));
        }
    }
    for (key, value) in spec.envs() {
        validate_environment_key(key)?;
        variables.insert(
            key.to_string_lossy().to_uppercase(),
            (key.to_owned(), value.to_owned()),
        );
    }

    let mut block = Vec::new();
    for (_, (key, value)) in variables {
        block.extend(key.encode_wide());
        block.push('=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }
    Ok(block)
}

/// Prevents malformed environment entries from changing the block's key/value boundaries.
fn validate_environment_key(key: &OsStr) -> io::Result<()> {
    let text = key.to_string_lossy();
    if text.is_empty() || text.contains(['=', '\0']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "environment variable name is empty or contains '=' or NUL",
        ));
    }
    Ok(())
}

/// Converts an OS string to a single NUL-terminated UTF-16 buffer.
pub(super) fn wide_nul(value: &OsStr, field: &str) -> io::Result<Vec<u16>> {
    let mut encoded = value.encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{field} contains NUL"),
        ));
    }
    encoded.push(0);
    Ok(encoded)
}

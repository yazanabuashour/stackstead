use std::{io::Write, path::Path};

use super::{
    model::{ComposeApplyOutput, HostBinding},
    planning::{plan_contents, resolve_compose_file},
    ports::detect_fixed_host_ports,
    yaml::port_declarations,
};

#[cfg(test)]
pub fn apply(repo_root: &Path) -> anyhow::Result<ComposeApplyOutput> {
    apply_at(repo_root, None)
}

pub fn apply_at(repo_root: &Path, requested: Option<&Path>) -> anyhow::Result<ComposeApplyOutput> {
    let repo_root = std::fs::canonicalize(repo_root)?;
    let path = resolve_compose_file(&repo_root, requested)?;
    let contents = std::fs::read_to_string(&path)?;
    let plan = plan_contents(&repo_root, &path, &contents)?;
    let document: serde_yaml::Value = serde_yaml::from_str(&contents)?;
    let declarations = port_declarations(&document, &path)?;
    let fixed = detect_fixed_host_ports(&contents);
    let planned_fixed = plan
        .ports
        .iter()
        .filter(|port| port.current_host_port.is_some())
        .count();
    if fixed.len() != planned_fixed {
        anyhow::bail!(
            "cannot safely rewrite all fixed host ports in {}; use one port mapping per YAML line",
            plan.file.display()
        );
    }
    let mut changed_lines = 0usize;
    let mut output = Vec::new();

    for (index, line) in contents.lines().enumerate() {
        let Some(fixed) = fixed
            .iter()
            .find(|fixed| fixed.file_line == index.saturating_add(1))
        else {
            output.push(line.to_owned());
            continue;
        };
        let matches = plan
            .ports
            .iter()
            .filter(|port| port.current_host_port == Some(fixed.host_port))
            .collect::<Vec<_>>();
        let [port] = matches.as_slice() else {
            anyhow::bail!(
                "cannot safely rewrite host port {} on line {}: expected one discovered service, found {}",
                fixed.host_port,
                fixed.file_line,
                matches.len()
            );
        };
        let replacement = if fixed.mapping.starts_with("published:") {
            format!("published: \"${{{}}}\"", port.env)
        } else {
            port.replacement.clone()
        };
        let updated = line.replacen(&fixed.mapping, &replacement, 1);
        if updated == line {
            anyhow::bail!(
                "could not safely locate the fixed host-port text on line {}",
                fixed.file_line
            );
        }
        output.push(updated);
        if fixed.mapping.starts_with("published:")
            && declarations.iter().any(|declaration| {
                declaration.host_binding == HostBinding::Fixed(fixed.host_port)
                    && declaration.host_ip.is_none()
            })
        {
            let indentation = line.strip_suffix(line.trim_start()).unwrap_or_default();
            output.push(format!("{indentation}host_ip: \"127.0.0.1\""));
        }
        changed_lines = changed_lines.saturating_add(1);
    }

    if changed_lines > 0 {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Compose path has no parent"))?;
        let permissions = std::fs::metadata(&path)?.permissions();
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(output.join("\n").as_bytes())?;
        if contents.ends_with('\n') {
            temporary.write_all(b"\n")?;
        }
        temporary.as_file().set_permissions(permissions)?;
        temporary.as_file().sync_all()?;
        if std::fs::read(&path)? != contents.as_bytes() {
            anyhow::bail!(
                "{} changed after Compose planning; no edits were applied",
                plan.file.display()
            );
        }
        temporary.persist(&path).map_err(|error| error.error)?;
    }
    Ok(ComposeApplyOutput {
        file: plan.file,
        changed_lines,
    })
}

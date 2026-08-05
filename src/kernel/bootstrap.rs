//! 数据根目录一次性初始化（docs/TODO.md：OOBE 初始化数据根目录）。
//!
//! `Kernel::new` 引导阶段与 `set_settings`（OOBE 完成路径）都会调用 `init_data_root`，
//! 幂等：创建固定子目录；`AGENTS.md` 缺失时写入默认教学规则模板（存在不覆盖）。

use std::path::Path;

/// 数据根目录下的固定子目录（与 ADR-0011 布局一致）。
const SUBDIRS: [&str; 6] = [
    "sessions",
    "mistakes",
    "memory",
    "audit",
    "logs",
    "uploads",
];

/// 默认教学规则模板：占位内容，家长/老师可编辑；仅首次创建时写入。
const AGENTS_MD_TEMPLATE: &str = r#"# 教学规则（AGENTS.md）

> 本文件是「错题 Agent」的教学规则，家长/老师可随时编辑；Agent 将按这些要求辅导学生。

## 通用要求
- 讲解耐心、清楚，使用中学生能听懂的语言，不展示思考过程。
- 批改作业先说明对错与得分，再重点讲解错题；错题自动归档进错题本。
- 数学、物理、化学等公式一律用 LaTeX 标记（行内 $...$，独立 $$...$$）。

## 分科要求
- 数学：强调解题步骤与错因分析。
- 物理 / 化学：强调公式适用条件与单位规范。
- 英语：讲解词汇与语法时给出例句。
- 语文：讲解阅读理解与作文思路。

## 家长/老师备注
（可在此补充对孩子的个性化要求，例如薄弱学科、答题规范、学习习惯等；不需要时可删除本段。）
"#;

/// 初始化数据根目录：创建根目录与固定子目录；`AGENTS.md` 缺失时写入默认模板（存在不覆盖）。
pub(crate) fn init_data_root(root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root)
        .map_err(|e| format!("创建数据根目录失败 {}：{e}", root.display()))?;
    for name in SUBDIRS {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("创建目录失败 {}：{e}", dir.display()))?;
    }
    let agents_md = root.join("AGENTS.md");
    if !agents_md.exists() {
        std::fs::write(&agents_md, AGENTS_MD_TEMPLATE)
            .map_err(|e| format!("写入 AGENTS.md 失败 {}：{e}", agents_md.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ma-bootstrap-{tag}-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn init_creates_subdirs_and_agents_md() {
        let root = tmp_root("create");
        init_data_root(&root).unwrap();
        for name in SUBDIRS {
            assert!(root.join(name).is_dir(), "缺少子目录 {name}");
        }
        let agents = root.join("AGENTS.md");
        assert!(agents.is_file());
        assert!(std::fs::read_to_string(&agents).unwrap().contains("教学规则"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn init_preserves_existing_agents_md() {
        let root = tmp_root("keep");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("AGENTS.md"), "家长自定义规则").unwrap();
        init_data_root(&root).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("AGENTS.md")).unwrap(),
            "家长自定义规则"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn init_is_idempotent() {
        let root = tmp_root("idem");
        init_data_root(&root).unwrap();
        init_data_root(&root).unwrap();
        for name in SUBDIRS {
            assert!(root.join(name).is_dir(), "缺少子目录 {name}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
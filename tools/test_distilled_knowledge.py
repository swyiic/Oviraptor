#!/usr/bin/env python3
"""Run production virtual knowledge resource code against in-memory fixtures."""
from pathlib import Path
import tempfile,subprocess
root=Path(__file__).resolve().parents[1];deps=root/'src-tauri/target/debug/deps'
source=root/'src-tauri/src/skill_packages/knowledge_resources.rs'
code='''#![allow(dead_code)]
mod package { #[path="SOURCE"] mod knowledge_resources;
#[cfg(test)] mod tests {use super::knowledge_resources::resources;use rusqlite::Connection;
fn db()->Connection{let d=Connection::open_in_memory().unwrap();d.execute_batch("CREATE TABLE skill_package_entries(skill_id INTEGER,entry_path TEXT,package_id TEXT); INSERT INTO skill_package_entries VALUES(1,'skills/a/SKILL.md','p1'),(2,'SKILL.md','p2'); CREATE TABLE skill_package_files(package_id TEXT,path TEXT); CREATE TABLE agent_knowledge_entries(id INTEGER,title TEXT,summary TEXT,skill_instructions TEXT,patterns_json TEXT,skill_id INTEGER,project_id INTEGER);").unwrap();d}
fn add(d:&Connection,id:i64,skill:i64,project:Option<i64>){d.execute("INSERT INTO agent_knowledge_entries VALUES(?1,'测试','summary','method','{\\\"sourceUrl\\\":\\\"https://example.invalid\\\"}',?2,?3)",rusqlite::params![id,skill,project]).unwrap();}
#[test]fn nested_package_reference_is_readable(){let d=db();add(&d,5,1,None);let r=resources(&d,1).unwrap();assert_eq!(r[0].0,"skills/a/references/distilled-5.md");let s=String::from_utf8(r[0].1.clone()).unwrap();assert!(s.contains("method")&&s.contains("https://example.invalid"));}
#[test]fn unrelated_skills_and_project_knowledge_are_excluded(){let d=db();add(&d,5,1,None);add(&d,6,2,None);add(&d,7,1,Some(3));assert_eq!(resources(&d,1).unwrap().len(),1);assert_eq!(resources(&d,2).unwrap().len(),1);}
#[test]fn imported_resource_cannot_shadow_knowledge(){let d=db();add(&d,5,1,None);d.execute("INSERT INTO skill_package_files VALUES('p1','skills/a/references/distilled-5.md')",[]).unwrap();assert_eq!(resources(&d,1).unwrap()[0].0,"skills/a/references/distilled-5-1.md");}
#[test]fn excessive_resources_fail_explicitly(){let d=db();for i in 0..129{add(&d,i,1,None)}assert!(resources(&d,1).is_err());}
#[test]fn old_database_without_knowledge_table_remains_readable(){let d=Connection::open_in_memory().unwrap();assert!(resources(&d,1).unwrap().is_empty());}
#[test]fn resource_changes_when_knowledge_changes(){let d=db();add(&d,5,1,None);let a=resources(&d,1).unwrap();d.execute("UPDATE agent_knowledge_entries SET skill_instructions='new method'",[]).unwrap();assert_ne!(a,resources(&d,1).unwrap());}
#[test]fn reclassification_removes_old_attachment(){let d=db();add(&d,5,1,None);d.execute("UPDATE agent_knowledge_entries SET skill_id=2",[]).unwrap();assert!(resources(&d,1).unwrap().is_empty());assert_eq!(resources(&d,2).unwrap()[0].0,"references/distilled-5.md");}
}}
'''.replace('SOURCE',str(source))
with tempfile.TemporaryDirectory(prefix='ovi-distilled-test-') as t:
 p=Path(t);(p/'test.rs').write_text(code);cmd=['rustc','--edition=2021','--test',str(p/'test.rs'),'-L',f'dependency={deps}','-o',str(p/'tests')]
 for name in ['rusqlite','serde_json']:
  lib=max(deps.glob(f'lib{name}-*.rlib'),key=lambda p:p.stat().st_mtime);cmd+=['--extern',f'{name}={lib}']
 subprocess.run(cmd,check=True);subprocess.run([str(p/'tests')],check=True)

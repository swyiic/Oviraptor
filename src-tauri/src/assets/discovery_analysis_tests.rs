use super::*;
fn fixture()->Connection{
 let c=Connection::open_in_memory().unwrap();
 c.execute_batch("CREATE TABLE projects(id INTEGER PRIMARY KEY,name TEXT,status TEXT);INSERT INTO projects VALUES(1,'移动项目','active'),(2,'其他项目','active');CREATE TABLE asset_ownership_profiles(project_id INTEGER,approved_domains_json TEXT);CREATE TABLE assets(id INTEGER PRIMARY KEY,host TEXT,link TEXT,company TEXT,extra_json TEXT,content_category TEXT);CREATE TABLE project_assets(project_id INTEGER,asset_id INTEGER,is_deleted INTEGER DEFAULT 0);").unwrap();ensure(&c).unwrap();c
}
fn add(c:&Connection,id:i64,p:i64,h:&str,company:&str,v:Value){c.execute("INSERT INTO assets VALUES(?1,?2,?2,?3,?4,'blocked_content')",params![id,format!("https://{h}"),company,v.to_string()]).unwrap();c.execute("INSERT INTO project_assets(project_id,asset_id) VALUES(?1,?2)",params![p,id]).unwrap();}
#[test]
fn report_scans_full_project_but_company_isp_and_raw_secrets_never_grant_ownership(){
 let c=fixture();add(&c,1,1,"one.example.com","中国移动集团",json!({"cert.domain":"cert.example.com","apiKey":"must-not-leak","body":"page-secret"}));
 add(&c,2,1,"unrelated.net","其他单位",json!({"isp":"中国移动","body":"中国移动"}));
 add(&c,3,2,"other.example.com","中国移动集团",json!({}));
 add(&c,4,1,"subject.example.net","",json!({"cert.subject.org":"中国移动集团","cname":"shared.vendor.net"}));
 let before=c.total_changes();let value=report_on(&c,1,"中国移动").unwrap();
 assert_eq!(value["scannedAssets"],3);assert_eq!(value["matchedAssets"],2);assert_eq!(before,c.total_changes());
 let text=value.to_string();for absent in ["must-not-leak","page-secret","unrelated.net","other.example.com"] {assert!(!text.contains(absent),"{absent}");}
 assert!(text.contains("shared.vendor.net"));for row in value["candidates"].as_array().unwrap(){assert_eq!(row["ownershipConfirmed"],false);assert_eq!(row["hunterAuthorized"],false);}
}
#[test]
fn confirmation_requires_explicit_operator_current_scope_and_saved_project_candidate(){
 let mut c=fixture();add(&c,1,1,"one.example.com","中国移动",json!({}));
 let input=|hosts:Vec<String>,expected:Vec<String>,confirmed|ConfirmInput{project_id:1,organization:"中国移动".into(),hosts,expected_roots:expected,user_confirmed:confirmed};
 assert!(confirm_on(&mut c,input(vec!["one.example.com".into()],vec![],false)).is_err());
 assert!(confirm_on(&mut c,input(vec!["invented.example.com".into()],vec![],true)).is_err());
 assert!(confirm_on(&mut c,input(vec!["one.example.com".into()],vec!["stale.example.com".into()],true)).is_err());
 assert!(roots(&c,1).unwrap().is_empty());
 let result=confirm_on(&mut c,input(vec!["one.example.com".into()],vec![],true)).unwrap();assert_eq!(result["hunterStarted"],false);assert_eq!(roots(&c,1).unwrap(),vec!["one.example.com"]);
}

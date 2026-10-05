// Called by the second of six tests. Each scenario invokes the actual capture path.
fn assert_helper_capture_limits_and_queue_gap() {
    const OUT: usize = 32 * 1024 * 1024;
    const ERR: usize = 64 * 1024;
    // Three boundary values prove complete Native machine JSON versus rejected truncation.
    for size in [OUT - 1, OUT, OUT + 1] {
        let f = HelperFixture::new();
        let control = f.control();
        let body = size - 12;
        let source=format!("require('fs').readFileSync(0);process.stdout.write(JSON.stringify({{body:'x'.repeat({body})}})+'\\n');");
        let worker = f.worker("fixture.cjs", &source);
        let result = run_native_json_helper_capture(
            &worker,
            &json!({}),
            &runtime(),
            None,
            "",
            Duration::from_secs(10),
            &control,
        );
        if size <= OUT {
            let captured = result.unwrap();
            let expected = format!("{{\"body\":\"{}\"}}\n", "x".repeat(body));
            assert_eq!(expected.len(), size);
            assert_eq!(captured.stdout, expected.as_bytes());
            assert!(!captured.stderr_truncated);
        } else {
            assert!(result.err().unwrap().contains("32 MiB"));
        }
        assert!(f.rows().iter().all(|r| r.hint.stream != "stdout"));
        assert_eq!(f.native(), "{\"bytes\":\"original-full-source-contract\"}");
    }
    // stderr remains byte exact up to 64KiB independently of ongoing safe diagnostic drain.
    for size in [ERR - 1, ERR, ERR + 1] {
        let f = HelperFixture::new();
        let control = f.control();
        let text = ("x".repeat(4095) + "\n").repeat(size / 4096).into_bytes();
        let mut text = text;
        text.extend_from_slice(&vec![b'x'; size % 4096]);
        let source=format!("require('fs').readFileSync(0);const raw=Buffer.from({});let n=0;const timer=setInterval(()=>{{if(n>=raw.length){{clearInterval(timer);process.stdout.write('{{\"ok\":true}}');}}else{{const end=Math.min(n+4096,raw.length);process.stderr.write(raw.subarray(n,end));n=end;}}}},12);",serde_json::to_string(&String::from_utf8(text.clone()).unwrap()).unwrap());
        let worker = f.worker("fixture.cjs", &source);
        let captured = run_native_json_helper_capture(
            &worker,
            &json!({}),
            &runtime(),
            None,
            "",
            Duration::from_secs(10),
            &control,
        )
        .unwrap();
        assert_eq!(captured.stderr, &text[..size.min(ERR)]);
        assert_eq!(captured.stderr_truncated, size > ERR);
        assert_eq!(captured.stdout, b"{\"ok\":true}");
    }
    // Real DB writer delays persistence while the real child fills the nonblocking queue.
    let f = HelperFixture::new();
    let control = f.control();
    let worker=f.worker("fixture.cjs",r#"
  const fs=require('fs'),path=require('path');fs.readFileSync(0);fs.writeFileSync(path.join(__dirname,'ready'),'yes');
  const timer=setInterval(()=>{if(fs.existsSync(path.join(__dirname,'burst'))){clearInterval(timer);fs.writeSync(2,'queue-pressure\n'.repeat(5000));
   const done=setInterval(()=>{if(fs.existsSync(path.join(__dirname,'release'))){clearInterval(done);process.stdout.write('{"ok":true}');}},5);}},5);
 "#);
    let root = f.root.clone();
    let path = f.db.clone();
    let lock = thread::spawn(move || {
        let wait = Instant::now();
        while !root.join("ready").exists() {
            assert!(wait.elapsed() < Duration::from_secs(3));
            thread::sleep(Duration::from_millis(5));
        }
        let c = db::open(&path).unwrap();
        let tx = c.unchecked_transaction().unwrap();
        tx.execute(
            "UPDATE sentinel_scans SET task_name=task_name WHERE id='web-a'",
            [],
        )
        .unwrap();
        fs::write(root.join("burst"), b"go").unwrap();
        thread::sleep(Duration::from_millis(200));
        tx.rollback().unwrap();
        fs::write(root.join("release"), b"go").unwrap();
    });
    let error = run_native_json_helper_capture(
        &worker,
        &json!({}),
        &runtime(),
        None,
        "",
        Duration::from_secs(8),
        &control,
    )
    .err()
    .unwrap();
    lock.join().unwrap();
    let c = db::open(&f.db).unwrap();
    let gaps: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM native_process_log_rows WHERE gap=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        gaps > 0,
        "actual helper queue pressure must be explicit durable loss: {error}"
    );
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM native_process_log_rows WHERE stream='status' AND message='gap'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

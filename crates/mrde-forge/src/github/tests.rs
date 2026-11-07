use super::*;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

/// A GraphQL pull request node shaped like GitHub's, with one approval among two reviews.
fn pull(number: u64, draft: bool) -> Value {
    json!({
        "number": number, "title": format!("Pull {number}"), "body": "Fixes #3",
        "url": format!("https://github.com/acme/platform/pull/{number}"),
        "createdAt": "2026-11-06T10:00:00Z", "isDraft": draft,
        "additions": 10, "deletions": 2, "author": {"login": "alice"},
        "headRefOid": "a".repeat(40), "baseRefOid": "b".repeat(40),
        "repository": {"databaseId": 42, "nameWithOwner": "acme/platform", "sshUrl": "git@github.com:acme/platform.git"},
        "labels": {"nodes": [{"name": "P1", "color": "d73a4a"}]},
        "latestOpinionatedReviews": {"nodes": [{"state": "APPROVED"}, {"state": "CHANGES_REQUESTED"}]},
        "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": "SUCCESS"}}}]}
    })
}
fn search(nodes: Vec<Value>) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"data": {"search": {
        "pageInfo": {"hasNextPage": false, "endCursor": null}, "nodes": nodes
    }}}))
}
async fn signed_in(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/api/v3/user"))
        .and(header("authorization", "Bearer token"))
        .and(header("x-github-api-version", "2022-11-28"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "reviewer"})))
        .mount(server)
        .await;
}

#[test]
fn urls_map_to_github_com_or_an_enterprise_api() {
    let public = GitHub::new("https://github.com/", "token").unwrap();
    assert_eq!(public.api.as_str(), "https://api.github.com/");
    assert_eq!(public.graphql.as_str(), "https://api.github.com/graphql");
    let enterprise = GitHub::new("https://ghe.example.com", "token").unwrap();
    assert_eq!(enterprise.api.as_str(), "https://ghe.example.com/api/v3/");
    assert_eq!(
        enterprise.graphql.as_str(),
        "https://ghe.example.com/api/graphql"
    );
    for invalid in [
        "http://ghe.example.com",
        "https://user:secret@github.com",
        "https://github.com/?token=1",
    ] {
        assert!(GitHub::new(invalid, "token").is_err(), "{invalid}");
    }
}

#[tokio::test]
async fn queue_searches_each_role_once_per_pull_and_preserves_drafts() {
    let server = MockServer::start().await;
    signed_in(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .and(body_string_contains("review-requested:reviewer"))
        .respond_with(search(vec![pull(7, false), pull(8, true), json!({})]))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .and(body_string_contains("reviewed-by:reviewer"))
        .respond_with(search(vec![pull(7, false), pull(9, false)]))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .and(body_string_contains("assignee:"))
        .respond_with(search(vec![]))
        .expect(0)
        .mount(&server)
        .await;
    let forge = GitHub::new(&server.uri(), "token").unwrap();
    let queue = forge.queue(&[MrRole::Reviewer]).await.unwrap();
    let ids: Vec<u64> = queue.iter().map(|mr| mr.id.iid).collect();
    assert_eq!(ids.len(), 3);
    assert!(
        ids.contains(&7) && ids.contains(&8) && ids.contains(&9),
        "{ids:?}"
    );
    assert!(queue.iter().find(|mr| mr.id.iid == 8).unwrap().draft);
    assert!(!queue.iter().find(|mr| mr.id.iid == 7).unwrap().draft);
    for request in server.received_requests().await.unwrap() {
        if request.url.path() == "/api/graphql" {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert!(
                !body["variables"]["q"]
                    .as_str()
                    .unwrap()
                    .contains("draft:false")
            );
        }
    }
    let mr = queue.iter().find(|mr| mr.id.iid == 7).unwrap();
    assert_eq!(mr.id.project, 42);
    assert_eq!(mr.project, "acme/platform");
    assert_eq!(mr.author, "alice");
    assert_eq!(mr.sha, "a".repeat(40));
    assert_eq!(mr.labels[0].color.as_deref(), Some("#d73a4a"));
    assert_eq!(mr.pipeline.as_deref(), Some("success"));
    assert_eq!(mr.approvals, Some(1));
    assert_eq!(mr.changed_lines, Some(12));
    // The refresh enriches each queued pull request right away: no request is needed.
    let summary = forge.summary(&mr.id).await.unwrap();
    assert_eq!(summary.title, "Pull 7");
}

#[tokio::test]
async fn owner_and_repository_scopes_use_search_qualifiers() {
    let server = MockServer::start().await;
    signed_in(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v3/users/acme"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"type": "Organization"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .and(body_string_contains("org:acme"))
        .respond_with(search(vec![pull(7, false)]))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .and(body_string_contains("repo:acme/platform"))
        .respond_with(search(vec![pull(9, false)]))
        .expect(1)
        .mount(&server)
        .await;
    let owner = GitHub::new(&server.uri(), "token")
        .unwrap()
        .with_group("acme");
    owner.test_group().await.unwrap();
    assert_eq!(owner.queue(&[MrRole::Author]).await.unwrap()[0].id.iid, 7);
    let repository = GitHub::new(&server.uri(), "token")
        .unwrap()
        .with_group("/acme/platform/");
    assert_eq!(
        repository.queue(&[MrRole::Author]).await.unwrap()[0].id.iid,
        9
    );
}

#[tokio::test]
async fn detail_lists_paged_files_the_merge_base_and_closing_issues() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repositories/42"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"full_name": "acme/platform"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let mut detail = pull(7, false);
    detail["closingIssuesReferences"] = json!({"nodes": [{
        "title": "Create recurring invoices", "body": "As an account manager…",
        "url": "https://github.com/acme/platform/issues/3",
        "repository": {"databaseId": 42},
        "labels": {"nodes": [{"name": "billing", "color": "0e8a16"}]}
    }]});
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .and(body_string_contains("closingIssuesReferences"))
        .and(body_partial_json(
            json!({"variables": {"owner": "acme", "name": "platform", "number": 7}}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data": {"repository": {"pullRequest": detail}}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repositories/42/pulls/7/files"))
        .and(query_param("page", "1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header(
                    "link",
                    "<https://api.github.com/repositories/42/pulls/7/files?page=2>; rel=\"next\"",
                )
                .set_body_json(json!([
                    {"filename": "src/new.rs", "previous_filename": "src/old.rs", "status": "renamed",
                     "patch": "@@ -1 +1 @@\n-old\n+new", "changes": 2}
                ])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repositories/42/pulls/7/files"))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"filename": "assets/logo.png", "status": "modified", "changes": 0},
            {"filename": "data/huge.json", "status": "modified", "changes": 9000}
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/api/v3/repositories/42/compare/{}...{}",
            "b".repeat(40),
            "a".repeat(40)
        )))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"merge_base_commit": {"sha": "c".repeat(40)}})),
        )
        .mount(&server)
        .await;
    let forge = GitHub::new(&server.uri(), "token").unwrap();
    let detail = forge
        .detail(&MrId {
            project: 42,
            iid: 7,
        })
        .await
        .unwrap();
    assert_eq!(
        detail.mr.base_sha,
        "c".repeat(40),
        "diffs start at the merge base"
    );
    assert_eq!(detail.ssh_url, "git@github.com:acme/platform.git");
    let paths: Vec<(&str, &str, bool)> = detail
        .files
        .iter()
        .map(|f| (f.old_path.as_str(), f.new_path.as_str(), f.too_large))
        .collect();
    assert_eq!(
        paths,
        [
            ("src/old.rs", "src/new.rs", false),
            ("assets/logo.png", "assets/logo.png", false),
            ("data/huge.json", "data/huge.json", true)
        ]
    );
    assert_eq!(detail.issues.len(), 1);
    assert_eq!(detail.issues[0].project_id, Some(42));
    assert_eq!(detail.mr.ticket_labels[0].name, "billing");
    assert_eq!(primary_tickets(&detail.mr, &detail.issues).len(), 1);
}

#[tokio::test]
async fn approval_requires_the_reviewed_head_and_sends_that_commit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repositories/42/pulls/7"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"head": {"sha": "a".repeat(40)}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v3/repositories/42/pulls/7/reviews"))
        .and(body_json(
            json!({"commit_id": "a".repeat(40), "event": "APPROVE"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 1})))
        .expect(1)
        .mount(&server)
        .await;
    let forge = GitHub::new(&server.uri(), "token").unwrap();
    let id = MrId {
        project: 42,
        iid: 7,
    };
    let changed = forge.approve(&id, &"d".repeat(40)).await.unwrap_err();
    assert!(changed.to_string().contains("changed since"), "{changed}");
    forge.approve(&id, &"a".repeat(40)).await.unwrap();
}

#[tokio::test]
async fn graphql_errors_and_rate_limits_are_reported() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/user"))
        .respond_with(ResponseTemplate::new(403).insert_header("x-ratelimit-remaining", "0"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repositories/42"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"full_name": "acme/platform"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": null, "errors": [{"message": "Resource not accessible by integration"}]
        })))
        .mount(&server)
        .await;
    let forge = GitHub::new(&server.uri(), "token").unwrap();
    let limited = forge.test_connection().await.unwrap_err();
    assert!(limited.to_string().contains("rate limit"), "{limited}");
    let error = forge
        .summary(&MrId {
            project: 42,
            iid: 7,
        })
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("Resource not accessible"),
        "{error}"
    );
}

#[tokio::test]
async fn overlaps_and_merged_history_use_open_and_recently_merged_pulls() {
    let server = MockServer::start().await;
    let recent = (Utc::now() - chrono::Duration::days(2)).to_rfc3339();
    let old = (Utc::now() - chrono::Duration::days(40)).to_rfc3339();
    let listed = |number: u64, merged: Option<&str>| {
        json!({"number": number, "title": format!("Pull {number}"),
            "html_url": format!("https://github.com/acme/platform/pull/{number}"),
            "updated_at": "2026-11-06T10:00:00Z", "merged_at": merged})
    };
    Mock::given(method("GET"))
        .and(path("/api/v3/repositories/42/pulls"))
        .and(query_param("state", "open"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!([listed(7, None), listed(8, None)])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repositories/42/pulls"))
        .and(query_param("state", "closed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            listed(5, Some(&recent)),
            listed(4, None),
            listed(3, Some(&old))
        ])))
        .mount(&server)
        .await;
    for (number, file) in [(8, "src/lib.rs"), (5, "src/other.rs"), (3, "src/lib.rs")] {
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v3/repositories/42/pulls/{number}/files"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!([{"filename": file, "patch": "@@ -1 +1 @@\n-a\n+b", "changes": 2}]),
            ))
            .mount(&server)
            .await;
    }
    let forge = GitHub::new(&server.uri(), "token").unwrap();
    let overlaps = forge
        .related(
            &MrId {
                project: 42,
                iid: 7,
            },
            &["src/lib.rs".into(), "src/other.rs".into()],
        )
        .await
        .unwrap();
    let found: Vec<(u64, bool)> = overlaps.iter().map(|o| (o.iid, o.merged)).collect();
    assert_eq!(found, [(5, true), (8, false)]);
    let history = forge.merged_history(42, 10).await.unwrap();
    let merged: Vec<u64> = history.iter().map(|id| id.iid).collect();
    assert_eq!(
        merged,
        [5, 3],
        "most recently merged first, closed-only excluded"
    );
}

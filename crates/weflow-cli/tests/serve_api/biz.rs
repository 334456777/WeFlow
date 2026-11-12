//! Official accounts and WeChat Pay records as the WebUI reads them.

use serde_json::json;

use super::common::{account_home_with, contract, misfits, start, temp_root, TOKEN};

#[test]
fn official_accounts_and_their_pushes_as_the_webui_reads_them() {
    use weflow_native::fixture::{MsgSpec, T0};
    let root = temp_root("biz");
    let push = |title: &str, more: &str| {
        format!(
            "<msg><appmsg><title>{title}</title><des>about {title}</des><url>https://mp.example/{title}</url>\
             <thumburl>https://mp.example/{title}.jpg</thumburl><content_list>{more}</content_list></appmsg></msg>"
        )
    };
    let second = "<item><title>second</title><url>https://mp.example/second</url><cover>https://mp.example/2.jpg</cover><digest>two</digest></item>";
    let home = account_home_with(&root, |f| {
        f.message_shard(
            0,
            &[(
                "gh_news",
                vec![
                    MsgSpec::text(1, "gh_news", T0, &push("first", "")).of_type(49),
                    MsgSpec::text(2, "gh_news", T0 + 3600, &push("lead", second)).of_type(49),
                ],
            )],
        );
    });
    let server = start(&home, &["--api-token", TOKEN]);

    let accounts = server.call("GET", "/biz/accounts", "").json();
    let accounts = accounts["data"]["accounts"].as_array().unwrap().clone();
    let news = accounts
        .iter()
        .find(|a| a["username"] == "gh_news")
        .unwrap_or_else(|| panic!("{accounts:?}"));
    assert_eq!(news["nick_name"], "News");

    let r = server.call(
        "GET",
        "/biz/messages?username=gh_news&limit=20&offset=0",
        "",
    );
    assert_eq!(r.status, 200, "{}", r.body);
    // as the contract says (#430)
    let fits = misfits(
        &contract(),
        &json!({ "$ref": "#/components/schemas/BizMessages" }),
        &r.json()["data"],
    );
    assert!(fits.is_empty(), "{fits:#?}");
    let pushes = r.json()["data"]["messages"].as_array().unwrap().clone();
    // newest first, the lead article and the others of the push
    let titles: Vec<&str> = pushes
        .iter()
        .map(|p| p["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["lead", "first"]);
    assert_eq!(pushes[0]["url"], "https://mp.example/lead");
    assert_eq!(pushes[0]["cover"], "https://mp.example/lead.jpg");
    assert_eq!(pushes[0]["description"], "about lead");
    assert_eq!(pushes[0]["contentList"][0]["title"], "second");
    assert_eq!(pushes[0]["contentList"][0]["summary"], "two");
    assert!(pushes[0]["createTime"].as_i64().unwrap() > pushes[1]["createTime"].as_i64().unwrap());
    let r = server.call(
        "GET",
        "/biz/messages?username=gh_news&limit=20&offset=1",
        "",
    );
    assert_eq!(r.json()["data"]["messages"].as_array().unwrap().len(), 1);
}

#[test]
fn wechat_pay_records_come_in_pages_that_say_whether_more_follow() {
    use weflow_native::fixture::{MsgSpec, T0};
    let root = temp_root("pay");
    let notice = |title: &str, des: &str, shop: &str| {
        format!(
            "<msg><appmsg><title>{title}</title><des>{des}</des><mmreader><template_header>\
             <display_name>{shop}</display_name><pub_time>{T0}</pub_time></template_header></mmreader></appmsg></msg>"
        )
    };
    const PAY: &str = "gh_3dfda90e39d6";
    let home = account_home_with(&root, |f| {
        f.biz_message_db(
            0,
            &[(
                PAY,
                vec![
                    MsgSpec::text(
                        1,
                        PAY,
                        T0,
                        &notice("微信支付凭证", "付款金额¥28.00\n支付方式零钱", "Cafe"),
                    )
                    .of_type(49),
                    // not a notice: left out, but still read
                    MsgSpec::text(
                        2,
                        PAY,
                        T0 + 60,
                        "<msg><appmsg><type>5</type></appmsg></msg>",
                    )
                    .of_type(49),
                    MsgSpec::text(3, PAY, T0 + 120, &notice("退款到账", "退款金额¥5.00", ""))
                        .of_type(49),
                ],
            )],
        );
    });
    let server = start(&home, &["--api-token", TOKEN]);

    let page = |offset: i32| {
        let r = server.call(
            "GET",
            &format!("/biz/pay-records?limit=2&offset={offset}"),
            "",
        );
        assert_eq!(r.status, 200, "{}", r.body);
        // as the contract says (#430)
        let fits = misfits(
            &contract(),
            &json!({ "$ref": "#/components/schemas/PayRecords" }),
            &r.json()["data"],
        );
        assert!(fits.is_empty(), "{fits:#?}");
        r.json()["data"].clone()
    };
    // newest first: the refund, then the message that is no notice
    let first = page(0);
    assert_eq!(first["hasMore"], true);
    let records = first["records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["title"], "退款到账");
    // no merchant named: WeChat Pay itself, in the server's language
    assert!(!records[0]["merchantName"].as_str().unwrap().is_empty());
    let second = page(2);
    assert_eq!(second["hasMore"], false);
    let records = second["records"].as_array().unwrap();
    assert_eq!(records[0]["merchantName"], "Cafe");
    assert_eq!(records[0]["description"], "付款金额¥28.00\n支付方式零钱");
    assert_eq!(records[0]["timestamp"], T0);
    // all of them at once
    let all = server.call("GET", "/biz/pay-records?limit=0", "").json();
    assert_eq!(all["data"]["records"].as_array().unwrap().len(), 2);
    assert_eq!(all["data"]["hasMore"], false);
}

#[test]
fn pay_records_include_the_cards_among_the_service_notices() {
    use weflow_native::fixture::{MsgSpec, T0};
    let root = temp_root("pay-cards");
    const PAY: &str = "gh_3dfda90e39d6";
    let card = |from: &str, title: &str| {
        format!(
            "<msg><fromusername>{from}</fromusername><appmsg><title>{title}</title><des>d</des></appmsg></msg>"
        )
    };
    let home = account_home_with(&root, |f| {
        f.biz_message_db(
            0,
            &[(
                "notifymessage",
                vec![
                    MsgSpec::text(1, "notifymessage", T0, &card(PAY, "付款")).of_type(49),
                    MsgSpec::text(2, "notifymessage", T0 + 60, &card("gh_other", "别的通知"))
                        .of_type(49),
                ],
            )],
        );
    });
    let server = start(&home, &["--api-token", TOKEN]);
    let all = server.call("GET", "/biz/pay-records?limit=0", "").json();
    let records = all["data"]["records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["title"], "付款");
    // a page counts the card of another account too
    let newest = server.call("GET", "/biz/pay-records?limit=1", "").json();
    assert_eq!(newest["data"]["records"].as_array().unwrap().len(), 0);
    assert_eq!(newest["data"]["hasMore"], true);
}

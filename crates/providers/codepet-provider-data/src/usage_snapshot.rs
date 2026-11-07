//! Frozen paging data, stored as typed columns and ordered child rows, never JSON blobs.
use crate::usage::{read_five, METRICS};
use crate::{db_error, error, ProtocolError};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

type Result<T> = std::result::Result<T, ProtocolError>;

#[derive(Debug, PartialEq)]
struct QueryBinding {
    instance: String,
    dataset: String,
    time_kind: String,
    from: Option<String>,
    to: Option<String>,
    time_zone: String,
    time_bucket: String,
    grouped: bool,
    include_unknown: Option<bool>,
    limit: u64,
    models: Option<Vec<String>>,
    metrics: Vec<String>,
    summaries: Option<Vec<String>>,
    orders: Option<Vec<(String, String)>>,
}
fn strings(value: &Value) -> Option<Vec<String>> {
    value.as_array().map(|v| {
        v.iter()
            .map(|v| v.as_str().unwrap_or_default().to_owned())
            .collect()
    })
}
impl QueryBinding {
    fn from_wire(instance: &str, q: &Value) -> Self {
        Self {
            instance: instance.into(),
            dataset: q["datasetId"].as_str().unwrap_or_default().into(),
            time_kind: q["filter"]["time"]["kind"]
                .as_str()
                .unwrap_or_default()
                .into(),
            from: q["filter"]["time"]["range"]["from"]
                .as_str()
                .map(str::to_owned),
            to: q["filter"]["time"]["range"]["to"]
                .as_str()
                .map(str::to_owned),
            time_zone: q["aggregation"]["timeZone"]
                .as_str()
                .unwrap_or_default()
                .into(),
            time_bucket: q["aggregation"]["timeBucket"]
                .as_str()
                .unwrap_or_default()
                .into(),
            grouped: q["aggregation"]["groupBy"]
                .as_array()
                .is_some_and(|a| !a.is_empty()),
            include_unknown: q["filter"]["includeUnknownModel"].as_bool(),
            limit: q["page"]["limit"].as_u64().unwrap_or(100),
            models: strings(&q["filter"]["modelIds"]),
            metrics: strings(&q["metrics"]).unwrap_or_default(),
            summaries: strings(&q["summaries"]),
            orders: q["orderBy"].as_array().map(|a| {
                a.iter()
                    .map(|o| {
                        (
                            o["field"].as_str().unwrap_or_default().into(),
                            o["direction"].as_str().unwrap_or_default().into(),
                        )
                    })
                    .collect()
            }),
        }
    }
    fn save(&self, db: &Connection, id: &str) -> Result<()> {
        db.execute("INSERT INTO usage_snapshot_queries VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![id,self.instance,self.dataset,self.time_kind,self.from,self.to,self.time_zone,self.time_bucket,self.grouped,self.include_unknown,self.limit,self.models.is_some(),self.summaries.is_some(),self.orders.is_some()]).map_err(db_error)?;
        for (kind, values) in [
            ("model", self.models.as_deref().unwrap_or_default()),
            ("metric", self.metrics.as_slice()),
            ("summary", self.summaries.as_deref().unwrap_or_default()),
        ] {
            for (index, value) in values.iter().enumerate() {
                db.execute(
                    "INSERT INTO usage_snapshot_query_options VALUES(?1,?2,?3,?4,NULL)",
                    params![id, kind, index, value],
                )
                .map_err(db_error)?;
            }
        }
        for (index, (field, direction)) in self
            .orders
            .as_deref()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            db.execute(
                "INSERT INTO usage_snapshot_query_options VALUES(?1,'order',?2,?3,?4)",
                params![id, index, field, direction],
            )
            .map_err(db_error)?;
        }
        Ok(())
    }
    fn load(db: &Connection, id: &str) -> Result<Option<Self>> {
        let mut query = db.query_row("SELECT instance,dataset,time_kind,range_from,range_to,time_zone,time_bucket,grouped,include_unknown,page_limit,has_models,has_summaries,has_orders FROM usage_snapshot_queries WHERE snapshot_id=?1",[id],|r|Ok(Self {
            instance:r.get(0)?,dataset:r.get(1)?,time_kind:r.get(2)?,from:r.get(3)?,to:r.get(4)?,time_zone:r.get(5)?,time_bucket:r.get(6)?,grouped:r.get(7)?,include_unknown:r.get(8)?,limit:r.get(9)?,
            models:r.get::<_,bool>(10)?.then(Vec::new),metrics:Vec::new(),summaries:r.get::<_,bool>(11)?.then(Vec::new),orders:r.get::<_,bool>(12)?.then(Vec::new),
        })).optional().map_err(db_error)?;
        if let Some(query) = &mut query {
            let mut statement = db.prepare("SELECT kind,value,direction FROM usage_snapshot_query_options WHERE snapshot_id=?1 ORDER BY kind,ordinal").map_err(db_error)?;
            for row in statement
                .query_map([id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(db_error)?
            {
                let (kind, value, direction) = row.map_err(db_error)?;
                match kind.as_str() {
                    "model" => query
                        .models
                        .as_mut()
                        .ok_or_else(|| db_error("Invalid model filter"))?
                        .push(value),
                    "metric" => query.metrics.push(value),
                    "summary" => query
                        .summaries
                        .as_mut()
                        .ok_or_else(|| db_error("Invalid summaries"))?
                        .push(value),
                    "order" => query
                        .orders
                        .as_mut()
                        .ok_or_else(|| db_error("Invalid order"))?
                        .push((
                            value,
                            direction.ok_or_else(|| db_error("Missing order direction"))?,
                        )),
                    _ => return Err(db_error("Invalid query option")),
                }
            }
        }
        Ok(query)
    }
}

const NATIVE: [&str; 5] = [
    "lifetimeTokens",
    "peakDailyTokens",
    "longestRunningTurnSec",
    "currentStreakDays",
    "longestStreakDays",
];
fn native_value(values: [Option<u64>; 5]) -> Value {
    let mut native = json!({});
    for (key, value) in NATIVE.iter().zip(values) {
        native[*key] = json!(value);
    }
    native
}
pub(crate) fn load_native(db: &Connection, instance: &str) -> Result<Option<Value>> {
    db.query_row("SELECT lifetime_tokens,peak_daily_tokens,longest_running_turn_seconds,current_streak_days,longest_streak_days FROM usage_native_summary WHERE instance=?1",[instance],|r|read_five(r,0))
        .optional().map(|v|v.map(native_value)).map_err(db_error)
}

pub(crate) fn save(
    db: &Connection,
    id: &str,
    instance: &str,
    q: &Value,
    result: &Value,
) -> Result<()> {
    let tx = db.unchecked_transaction().map_err(db_error)?;
    let summary = &result["summaries"];
    let flags = i64::from(result.get("summaries").is_some())
        | (i64::from(summary.get("totals").is_some()) << 1)
        | (i64::from(summary.get("totalsByGroup").is_some()) << 2)
        | (i64::from(summary.get("peakDaily").is_some()) << 3)
        | (i64::from(summary.get("peakDailyByGroup").is_some()) << 4);
    tx.execute(
        "INSERT INTO usage_snapshots VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            id,
            result["datasetId"].as_str(),
            result["generatedAt"].as_str(),
            result["updatedAt"].as_str(),
            result["coverage"]["scope"].as_str(),
            result["coverage"]["availableFrom"].as_str(),
            result["coverage"]["completeThrough"].as_str(),
            result["coverage"]["status"].as_str(),
            flags,
            chrono::Utc::now().timestamp() + 300
        ],
    )
    .map_err(db_error)?;
    QueryBinding::from_wire(instance, q).save(&tx, id)?;
    for (kind, values) in [
        ("row", &result["rows"]),
        ("group_total", &summary["totalsByGroup"]),
        ("group_peak", &summary["peakDailyByGroup"]),
    ] {
        if let Some(values) = values.as_array() {
            for (index, value) in values.iter().enumerate() {
                save_entry(&tx, id, kind, index, value)?;
            }
        }
    }
    if let Some(total) = summary.get("totals") {
        save_entry(&tx, id, "total", 0, &json!({"values":total}))?;
    }
    if let Some(peak) = summary.get("peakDaily").filter(|v| !v.is_null()) {
        save_entry(&tx, id, "peak", 0, peak)?;
    }
    if let Some(native) = result.get("nativeAccountSummary") {
        let v: [Option<u64>; 5] = std::array::from_fn(|i| native[NATIVE[i]].as_u64());
        tx.execute(
            "INSERT INTO usage_snapshot_accounts VALUES(?1,?2,?3,?4,?5,?6)",
            params![id, v[0], v[1], v[2], v[3], v[4]],
        )
        .map_err(db_error)?;
    }
    tx.commit().map_err(db_error)
}
fn save_entry(db: &Connection, id: &str, kind: &str, index: usize, item: &Value) -> Result<()> {
    let values: [Option<u64>; 5] =
        std::array::from_fn(|i| item["values"][METRICS[i]]["value"].as_u64());
    let completeness: [Option<&str>; 5] =
        std::array::from_fn(|i| item["values"][METRICS[i]]["completeness"].as_str());
    let peak = if kind == "group_peak" {
        &item["peak"]
    } else {
        item
    };
    db.execute("INSERT INTO usage_snapshot_entries VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",params![
        id,kind,index,item.get("modelId").is_some(),item["modelId"].as_str(),item["bucket"]["from"].as_str(),item["bucket"]["to"].as_str(),
        item["provisional"].as_bool(),item["completeness"].as_str(),peak["date"].as_str(),peak["totalTokens"].as_u64(),
        values[0],values[1],values[2],values[3],values[4],completeness[0],completeness[1],completeness[2],completeness[3],completeness[4]
    ]).map_err(db_error)?;
    Ok(())
}
pub(crate) fn load(db: &Connection, id: &str, instance: &str, q: &Value) -> Result<Value> {
    // Pin one read snapshot while another query can expire/prune saved pages.
    let transaction = db.unchecked_transaction().map_err(db_error)?;
    let db = &*transaction;
    let expected = QueryBinding::load(db, id)?
        .ok_or_else(|| error("usage_cursor_expired", "Usage snapshot expired"))?;
    if expected != QueryBinding::from_wire(instance, q) {
        return Err(error(
            "invalid_cursor",
            "Cursor does not match query or instance",
        ));
    }
    let (mut result,flags) = db.query_row("SELECT dataset,generated_at,updated_at,scope,available_from,complete_through,coverage_status,summary_flags FROM usage_snapshots WHERE id=?1",[id],|r| {
        Ok((json!({"datasetId":r.get::<_,String>(0)?,"revision":id,"generatedAt":r.get::<_,String>(1)?,"updatedAt":r.get::<_,Option<String>>(2)?,
            "coverage":{"scope":r.get::<_,String>(3)?,"availableFrom":r.get::<_,Option<String>>(4)?,"completeThrough":r.get::<_,Option<String>>(5)?,"status":r.get::<_,String>(6)?,"gaps":[]},
            "rows":[],"nextCursor":null}),r.get::<_,i64>(7)?))
    }).map_err(db_error)?;
    if flags & 1 != 0 {
        result["summaries"] = json!({});
    }
    if flags & 2 != 0 {
        result["summaries"]["totals"] = json!({});
    }
    if flags & 4 != 0 {
        result["summaries"]["totalsByGroup"] = json!([]);
    }
    if flags & 8 != 0 {
        result["summaries"]["peakDaily"] = Value::Null;
    }
    if flags & 16 != 0 {
        result["summaries"]["peakDailyByGroup"] = json!([]);
    }
    let mut statement = db.prepare("SELECT kind,model_present,model_id,bucket_from,bucket_to,provisional,completeness,date,peak_total,total_tokens,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,total_completeness,input_completeness,output_completeness,cache_read_completeness,cache_write_completeness FROM usage_snapshot_entries WHERE snapshot_id=?1 ORDER BY kind,ordinal").map_err(db_error)?;
    let rows = statement.query_map([id],|r| {
        let kind:String = r.get(0)?;
        let mut item = json!({});
        if r.get::<_,bool>(1)? { item["modelId"]=json!(r.get::<_,Option<String>>(2)?); }
        if kind=="row" {
            item["bucket"] = match r.get::<_,Option<String>>(3)? { Some(from)=>json!({"from":from,"to":r.get::<_,String>(4)?}),None=>Value::Null };
            item["provisional"]=json!(r.get::<_,bool>(5)?);
            item["completeness"]=json!(r.get::<_,String>(6)?);
        }
        if ["row","total","group_total"].contains(&kind.as_str()) {
            let values:[Option<u64>;5]=read_five(r,9)?;
            let completeness:[Option<String>;5]=read_five(r,14)?;
            item["values"]=json!({});
            for i in 0..5 { if let Some(c)=&completeness[i] { item["values"][METRICS[i]]=json!({"value":values[i],"completeness":c}); } }
        } else {
            let peak = match r.get::<_,Option<String>>(7)? {
                Some(date)=>json!({"date":date,"totalTokens":r.get::<_,u64>(8)?,"completeness":"partial"}),None=>Value::Null,
            };
            if kind=="group_peak" { item["peak"]=peak; } else { item=peak; }
        }
        Ok((kind,item))
    }).map_err(db_error)?;
    for row in rows {
        let (kind, item) = row.map_err(db_error)?;
        match kind.as_str() {
            "row" => result["rows"].as_array_mut().unwrap().push(item),
            "total" => result["summaries"]["totals"] = item["values"].clone(),
            "group_total" => result["summaries"]["totalsByGroup"]
                .as_array_mut()
                .unwrap()
                .push(item),
            "peak" => result["summaries"]["peakDaily"] = item,
            "group_peak" => result["summaries"]["peakDailyByGroup"]
                .as_array_mut()
                .unwrap()
                .push(item),
            _ => return Err(db_error("Invalid snapshot entry kind")),
        }
    }
    let native = db.query_row("SELECT lifetime_tokens,peak_daily_tokens,longest_running_turn_seconds,current_streak_days,longest_streak_days FROM usage_snapshot_accounts WHERE snapshot_id=?1",[id],|r|read_five(r,0)).optional().map_err(db_error)?;
    if let Some(native) = native {
        result["nativeAccountSummary"] = native_value(native);
    }
    Ok(result)
}

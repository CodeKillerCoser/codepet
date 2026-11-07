use crate::{db_error, error, ProviderData};
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use chrono_tz::Tz;
use codepet_provider_sdk::{ProtocolError, UsageDataset, UsageQuery, UsageQueryResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const METRICS: [&str; 5] = [
    "totalTokens",
    "inputTokens",
    "outputTokens",
    "cacheReadTokens",
    "cacheWriteTokens",
];
const MAX_TOKENS: u64 = 9_007_199_254_740_991;
pub const HOOK_DATASET: &str = "observed-model-tokens";
pub const CODEX_DATASET: &str = "codex-account-daily";

pub(crate) fn initialize(db: &Connection) -> Result<(), ProtocolError> {
    db.execute_batch(include_str!("usage.sql")).map_err(db_error)
}
pub(crate) fn read_five<T: rusqlite::types::FromSql>(r: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<[T;5]> {
    Ok([r.get(offset)?,r.get(offset+1)?,r.get(offset+2)?,r.get(offset+3)?,r.get(offset+4)?])
}

pub fn datasets(codex: bool) -> Vec<UsageDataset> {
    let value = if codex {
        json!({"id":CODEX_DATASET,"displayName":"Codex account daily tokens","scope":"account",
            "metrics":["totalTokens"],"timeBuckets":["none","day","month"],"modelFilter":false,
            "modelGrouping":false,"timeZones":["UTC"],"baseBucketMinutes":1440})
    } else {
        json!({"id":HOOK_DATASET,"displayName":"Observed model tokens","scope":"local",
            "metrics":METRICS,"timeBuckets":["none","halfHour","hour","day","month"],
            "modelFilter":true,"modelGrouping":true,"timeZones":[],"baseBucketMinutes":30})
    };
    vec![serde_json::from_value(value).expect("static usage capability")]
}

impl ProviderData {
    /// Identity belongs to the source request/message, never a Hook delivery ID.
    /// Re-delivery is an upsert, so result notifications and historical rereads cannot double count.
    pub fn record(
        &self,
        instance: &str,
        dataset: &str,
        id: &str,
        at: i64,
        duration: i64,
        model: Option<&str>,
        values: [Option<u64>; 5],
    ) -> Result<(), ProtocolError> {
        if id.is_empty()
            || at < 0
            || duration <= 0
            || at.checked_add(duration).is_none_or(|t| t > 253402214400)
            || values.iter().flatten().any(|n| *n > MAX_TOKENS)
        {
            return Err(error(
                "invalid_usage_record",
                "Invalid identity or token count",
            ));
        }
        if let (Some(total), Some(input), Some(output)) = (values[0], values[1], values[2]) {
            if input.checked_add(output) != Some(total) {
                return Err(error(
                    "invalid_usage_record",
                    "Total must equal input plus output",
                ));
            }
        }
        let mut db = self.connection()?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let previous:Option<(i64,i64,Option<String>,[Option<u64>;5])>=tx.query_row(
            "SELECT at,duration,model,total_tokens,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens FROM usage_records WHERE instance=?1 AND dataset=?2 AND record_id=?3",
            params![instance,dataset,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,read_five(r,3)?))).optional().map_err(db_error)?;
        if previous.as_ref().is_some_and(|(time, span, m, v)| {
            *time == at && *span == duration && m.as_deref() == model && *v == values
        }) {
            return Ok(());
        }
        if let Some((time, span, m, v)) = previous {
            adjust_bucket(
                &tx,
                instance,
                dataset,
                time,
                span,
                m.as_deref(),
                v,
                -1,
            )?;
        }
        adjust_bucket(&tx, instance, dataset, at, duration, model, values, 1)?;
        tx.execute("INSERT INTO usage_records VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
            ON CONFLICT(instance,dataset,record_id) DO UPDATE SET at=excluded.at,duration=excluded.duration,
            model=excluded.model,total_tokens=excluded.total_tokens,input_tokens=excluded.input_tokens,output_tokens=excluded.output_tokens,cache_read_tokens=excluded.cache_read_tokens,cache_write_tokens=excluded.cache_write_tokens,updated=excluded.updated",
            params![instance,dataset,id,at,duration,model,values[0],values[1],values[2],values[3],values[4],Utc::now().timestamp_millis()]).map_err(db_error)?;
        tx.commit().map_err(db_error)?;
        Ok(())
    }

    pub fn query(
        &self,
        instance: &str,
        query: UsageQuery,
        codex: bool,
    ) -> Result<UsageQueryResult, ProtocolError> {
        self.query_in_scope(instance, instance, query, codex)
    }
    pub fn query_observed(
        &self,
        instance: &str,
        query: UsageQuery,
    ) -> Result<UsageQueryResult, ProtocolError> {
        self.query_in_scope(instance, "observation", query, false)
    }
    fn query_in_scope(
        &self,
        instance: &str,
        source: &str,
        query: UsageQuery,
        codex: bool,
    ) -> Result<UsageQueryResult, ProtocolError> {
        let q = serde_json::to_value(query).map_err(db_error)?;
        let dataset = if codex { CODEX_DATASET } else { HOOK_DATASET };
        if q["datasetId"] != dataset {
            return Err(error("unknown_usage_dataset", "Unknown dataset"));
        }
        let tz: Tz = q["aggregation"]["timeZone"]
            .as_str()
            .unwrap_or("")
            .parse()
            .map_err(|_| error("invalid_usage_query", "Invalid IANA timezone"))?;
        let grain = q["aggregation"]["timeBucket"].as_str().unwrap_or("none");
        let grouped = q["aggregation"]["groupBy"]
            .as_array()
            .is_some_and(|v| !v.is_empty());
        if q["aggregation"]["groupBy"]
            .as_array()
            .is_some_and(|v| v.len() > 1)
        {
            return Err(error("invalid_usage_query", "Duplicate grouping dimension"));
        }
        let models = q["filter"]["modelIds"].as_array();
        if models.is_some_and(|v| {
            v.is_empty()
                || v.len() > 256
                || v.iter()
                    .any(|m| m.as_str().is_none_or(|s| s.is_empty() || s.len() > 1024))
                || v.iter()
                    .filter_map(Value::as_str)
                    .collect::<BTreeSet<_>>()
                    .len()
                    != v.len()
        }) {
            return Err(error(
                "invalid_usage_query",
                "modelIds must contain 1..256 model IDs",
            ));
        }
        if codex
            && (tz != chrono_tz::UTC
                || grouped
                || models.is_some()
                || !["none", "day", "month"].contains(&grain))
        {
            return Err(error(
                "unsupported_usage_query",
                "Codex account data supports UTC daily/monthly totals only",
            ));
        }
        let metrics = q["metrics"]
            .as_array()
            .ok_or_else(|| error("invalid_usage_query", "Missing metrics"))?;
        if metrics.is_empty()
            || metrics.len() > 5
            || metrics
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
                .len()
                != metrics.len()
        {
            return Err(error(
                "invalid_usage_query",
                "Metrics must be unique and nonempty",
            ));
        }
        if codex && metrics.iter().any(|v| v != "totalTokens") {
            return Err(error(
                "unsupported_usage_query",
                "Codex account data has no token breakdown",
            ));
        }
        let summaries = q["summaries"].as_array().cloned().unwrap_or_default();
        if summaries
            .iter()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>()
            .len()
            != summaries.len()
        {
            return Err(error("invalid_usage_query", "Duplicate summary"));
        }
        if !grouped
            && summaries
                .iter()
                .any(|v| v.as_str().unwrap_or("").ends_with("ByGroup"))
        {
            return Err(error(
                "invalid_usage_query",
                "Group summaries require model grouping",
            ));
        }
        let precision = if codex { 86400 } else { 1800 };
        let range = &q["filter"]["time"];
        let (from, to) = if range["kind"] == "range" {
            let from = parse_time(&range["range"]["from"])?;
            let to = parse_time(&range["range"]["to"])?;
            if from < 0
                || to > 253402214400
                || from >= to
                || from.rem_euclid(precision) != 0
                || to.rem_euclid(precision) != 0
            {
                return Err(error(
                    "unsupported_usage_query",
                    "Range must be nonempty and aligned to source bucket boundaries",
                ));
            }
            (from, to)
        } else {
            (0, 253402300799)
        };
        let limit = q["page"]["limit"].as_u64().unwrap_or(100) as usize;
        if !(1..=1000).contains(&limit) {
            return Err(error("invalid_usage_query", "Invalid page limit"));
        }
        let db = self.connection()?;
        db.execute("DELETE FROM usage_snapshots WHERE expires < ?1", [Utc::now().timestamp()]).map_err(db_error)?;
        if let Some(cursor) = q["page"]["cursor"].as_str() {
            let (id,offset) = cursor.split_once(':').ok_or_else(||error("invalid_cursor","Malformed usage cursor"))?;
            let offset = offset.parse().map_err(|_|error("invalid_cursor","Malformed offset"))?;
            return page_result(crate::usage_snapshot::load(&db,id,instance,&q)?, id,offset,limit);
        }
        let mut statement = db.prepare("SELECT at,duration,NULLIF(model,''),total_tokens,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,updated,total_tokens_known_count,input_tokens_known_count,output_tokens_known_count,cache_read_tokens_known_count,cache_write_tokens_known_count,records FROM usage_buckets WHERE instance=?1 AND dataset=?2 AND at>=?3 AND at<?4 ORDER BY at,model").map_err(db_error)?;
        let records = statement.query_map(params![source,dataset,from,to], |r| Ok((
            r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,Option<String>>(2)?,read_five::<u64>(r,3)?,r.get::<_,i64>(8)?,read_five::<i64>(r,9)?,r.get::<_,i64>(14)?
        ))).map_err(db_error)?;
        let mut buckets: BTreeMap<(i64, Option<String>), (i64, Sums)> = BTreeMap::new();
        let mut daily: BTreeMap<(String, Option<String>), Sums> = BTreeMap::new();
        let mut total = Sums::default();
        let mut group_totals: BTreeMap<Option<String>, Sums> = BTreeMap::new();
        let mut updated = None;
        let mut available = None;
        for record in records {
            let (at, duration, model, values, last, known, count) = record.map_err(db_error)?;
            if model.is_none() {
                if !q["filter"]["includeUnknownModel"]
                    .as_bool()
                    .unwrap_or(models.is_none())
                {
                    continue;
                }
            } else if let Some(models) = models {
                if !model
                    .as_ref()
                    .is_some_and(|m| models.iter().any(|v| v.as_str() == Some(m.as_str())))
                {
                    continue;
                }
            }
            let sums = values;
            let values = std::array::from_fn(|i| (known[i] > 0).then_some(sums[i]));
            let missing = std::array::from_fn(|i| known[i] < count);
            let (start, end) = bucket(at, grain, tz)?;
            // A source bucket may not be split by a timezone or requested grain.
            if grain != "none" && (at < start || at + duration > end) {
                return Err(error(
                    "unsupported_usage_query",
                    "Source bucket crosses requested timezone boundary",
                ));
            }
            let key_model = if grouped { model.clone() } else { None };
            let row_end = if grain == "none" { at + duration } else { end };
            let row = buckets
                .entry((start, key_model.clone()))
                .or_insert_with(|| (row_end, Sums::default()));
            row.0 = row.0.max(row_end);
            row.1.add_bucket(values, missing)?;
            total.add_bucket(values, missing)?;
            group_totals
                .entry(key_model.clone())
                .or_default()
                .add_bucket(values, missing)?;
            let date = Utc
                .timestamp_opt(at, 0)
                .single()
                .ok_or_else(|| error("invalid_usage_record", "Timestamp out of range"))?
                .with_timezone(&tz)
                .format("%Y-%m-%d")
                .to_string();
            daily
                .entry((date.clone(), key_model))
                .or_default()
                .add_bucket(values, missing)?;
            updated = Some(updated.unwrap_or(0).max(last));
            available = Some(available.unwrap_or(at).min(at));
            if buckets.len() > 100_000 {
                return Err(error(
                    "usage_query_too_large",
                    "Narrow the time range or use coarser grouping",
                ));
            }
        }
        let mut rows: Vec<Value> = buckets.into_iter().map(|((start,model),(end,sums))| {
            let mut row = json!({"bucket":if grain=="none" {Value::Null} else {json!({"from":iso(start),"to":iso(end)})},
                "values":sums.values(metrics),"completeness":"partial","provisional":end>Utc::now().timestamp()});
            if grouped { row["modelId"]=json!(model); } row
        }).collect();
        if let Some(orders) = q["orderBy"].as_array() {
            let mut fields = BTreeSet::new();
            for order in orders {
                let field = order["field"].as_str().unwrap_or("");
                if !fields.insert(field)
                    || (field == "modelId" && !grouped)
                    || (field == "bucketStart" && grain == "none")
                    || (METRICS.contains(&field) && !metrics.iter().any(|m| m == field))
                {
                    return Err(error(
                        "invalid_usage_query",
                        "Invalid or duplicate sort field",
                    ));
                }
            }
            rows.sort_by(|a, b| {
                for o in orders {
                    let field = o["field"].as_str().unwrap();
                    let comparison = match field {
                        "bucketStart" => a["bucket"]["from"]
                            .as_str()
                            .cmp(&b["bucket"]["from"].as_str()),
                        "modelId" => a["modelId"].as_str().cmp(&b["modelId"].as_str()),
                        _ => a["values"][field]["value"]
                            .as_u64()
                            .cmp(&b["values"][field]["value"].as_u64()),
                    };
                    if !comparison.is_eq() {
                        return if o["direction"] == "desc" {
                            comparison.reverse()
                        } else {
                            comparison
                        };
                    }
                }
                std::cmp::Ordering::Equal
            });
        }
        let mut summary = json!({});
        if summaries.iter().any(|v| v == "totals") {
            summary["totals"] = total.values(metrics);
        }
        if summaries.iter().any(|v| v == "totalsByGroup") {
            summary["totalsByGroup"] = json!(group_totals
                .iter()
                .map(|(m, s)| json!({"modelId":m,"values":s.values(metrics)}))
                .collect::<Vec<_>>());
        }
        if summaries.iter().any(|v| v == "peakDaily") {
            let mut days: BTreeMap<String, u64> = BTreeMap::new();
            for ((date, _), s) in &daily {
                if let Some(n) = s.counts[0] {
                    let v = days.entry(date.clone()).or_default();
                    *v = v
                        .checked_add(n)
                        .filter(|v| *v <= MAX_TOKENS)
                        .ok_or_else(|| {
                            error("usage_overflow", "Token sum exceeds JSON safe integer")
                        })?;
                }
            }
            summary["peakDaily"] = peak(days);
        }
        if summaries.iter().any(|v| v == "peakDailyByGroup") {
            summary["peakDailyByGroup"]=json!(group_totals.keys().map(|model|json!({"modelId":model,"peak":peak(daily.iter().filter(|((_,m),_)|m==model).filter_map(|((d,_),s)|s.counts[0].map(|n|(d.clone(),n))).collect())})).collect::<Vec<_>>());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let mut result = json!({"datasetId":dataset,"revision":id,"generatedAt":Utc::now().to_rfc3339(),
            "updatedAt":updated.map(|t|iso(t/1000)),"coverage":{"scope":if codex {"account"} else {"local"},
            "availableFrom":available.map(iso),"completeThrough":null,"status":"partial","gaps":[]},
            "rows":rows,"nextCursor":null});
        if !summaries.is_empty() {
            result["summaries"] = summary;
        }
        if codex {
            if let Some(native) = crate::usage_snapshot::load_native(&db,source)? {
                result["nativeAccountSummary"] = native;
            }
        }

        crate::usage_snapshot::save(&db,&id,instance,&q,&result)?;
        db.execute("DELETE FROM usage_snapshots WHERE id NOT IN (SELECT id FROM usage_snapshots ORDER BY expires DESC LIMIT 32)",[]).map_err(db_error)?;
        page_result(result, &id, 0, limit)
    }
}

#[derive(Default)]
struct Sums {
    counts: [Option<u64>; 5],
    missing: [bool; 5],
}
impl Sums {
    fn add_bucket(&mut self, v: [Option<u64>; 5], missing: [bool; 5]) -> Result<(), ProtocolError> {
        self.add(v)?;
        for i in 0..5 {
            self.missing[i] |= missing[i];
        }
        Ok(())
    }
    fn add(&mut self, v: [Option<u64>; 5]) -> Result<(), ProtocolError> {
        for i in 0..5 {
            match v[i] {
                Some(n) => {
                    self.counts[i] = Some(
                        self.counts[i]
                            .unwrap_or(0)
                            .checked_add(n)
                            .filter(|n| *n <= MAX_TOKENS)
                            .ok_or_else(|| {
                                error("usage_overflow", "Token sum exceeds JSON safe integer")
                            })?,
                    )
                }
                None => self.missing[i] = true,
            }
        }
        Ok(())
    }
    fn values(&self, metrics: &[Value]) -> Value {
        let mut result = json!({});
        for (i, key) in METRICS.iter().enumerate() {
            if metrics.iter().any(|v| v == key) {
                result[key] = json!({"value":self.counts[i],"completeness":if self.counts[i].is_none(){"unknown"}else if self.missing[i]{"partial"}else{"complete"}});
            }
        }
        result
    }
}
fn adjust_bucket(
    db: &Connection,
    instance: &str,
    dataset: &str,
    at: i64,
    duration: i64,
    model: Option<&str>,
    values: [Option<u64>; 5],
    delta: i64,
) -> Result<(), ProtocolError> {
    let old:Option<([u64;5],[i64;5],i64)>=db.query_row("SELECT total_tokens,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,total_tokens_known_count,input_tokens_known_count,output_tokens_known_count,cache_read_tokens_known_count,cache_write_tokens_known_count,records FROM usage_buckets WHERE instance=?1 AND dataset=?2 AND at=?3 AND model=?4",params![instance,dataset,at,model.unwrap_or("")],|r|Ok((read_five(r,0)?,read_five(r,5)?,r.get(10)?))).optional().map_err(db_error)?;
    let (mut sums,mut known,mut records) = old.unwrap_or(([0;5],[0;5],0));
    records += delta;
    for i in 0..5 {
        if let Some(n) = values[i] {
            known[i] += delta;
            sums[i] = if delta > 0 {
                sums[i].checked_add(n).filter(|n| *n <= MAX_TOKENS)
            } else {
                sums[i].checked_sub(n)
            }
            .ok_or_else(|| error("usage_overflow", "Invalid aggregate token count"))?;
        }
    }
    if records == 0 {
        db.execute(
            "DELETE FROM usage_buckets WHERE instance=?1 AND dataset=?2 AND at=?3 AND model=?4",
            params![instance, dataset, at, model.unwrap_or("")],
        )
        .map_err(db_error)?;
    } else {
        db.execute("INSERT INTO usage_buckets VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17) ON CONFLICT(instance,dataset,at,model) DO UPDATE SET total_tokens=excluded.total_tokens,input_tokens=excluded.input_tokens,output_tokens=excluded.output_tokens,cache_read_tokens=excluded.cache_read_tokens,cache_write_tokens=excluded.cache_write_tokens,total_tokens_known_count=excluded.total_tokens_known_count,input_tokens_known_count=excluded.input_tokens_known_count,output_tokens_known_count=excluded.output_tokens_known_count,cache_read_tokens_known_count=excluded.cache_read_tokens_known_count,cache_write_tokens_known_count=excluded.cache_write_tokens_known_count,records=excluded.records,updated=excluded.updated",params![instance,dataset,at,duration,model.unwrap_or(""),sums[0],sums[1],sums[2],sums[3],sums[4],known[0],known[1],known[2],known[3],known[4],records,Utc::now().timestamp_millis()]).map_err(db_error)?;
    }
    Ok(())
}
fn parse_time(v: &Value) -> Result<i64, ProtocolError> {
    let t = DateTime::parse_from_rfc3339(v.as_str().unwrap_or(""))
        .map_err(|_| error("invalid_usage_query", "Invalid timestamp"))?;
    if t.timestamp_subsec_nanos() != 0 {
        return Err(error(
            "unsupported_usage_query",
            "Subsecond boundaries are unsupported",
        ));
    }
    Ok(t.timestamp())
}
fn iso(t: i64) -> String {
    Utc.timestamp_opt(t, 0)
        .single()
        .expect("validated timestamp")
        .to_rfc3339()
}
fn bucket(at: i64, grain: &str, tz: Tz) -> Result<(i64, i64), ProtocolError> {
    let t = Utc
        .timestamp_opt(at, 0)
        .single()
        .ok_or_else(|| error("invalid_usage_record", "Invalid timestamp"))?
        .with_timezone(&tz);
    if grain == "none" {
        return Ok((0, 253402300799));
    }
    if grain == "halfHour" || grain == "hour" {
        let size = if grain == "hour" { 3600 } else { 1800 };
        let offset = t.offset().fix().local_minus_utc() as i64;
        let start = (at + offset).div_euclid(size) * size - offset;
        return Ok((start, start + size));
    }
    let day = t.date_naive();
    let (start, end) = if grain == "month" {
        let start = day.with_day(1).unwrap();
        let end = if start.month() == 12 {
            chrono::NaiveDate::from_ymd_opt(start.year() + 1, 1, 1)
        } else {
            chrono::NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1)
        }
        .ok_or_else(|| error("invalid_usage_query", "Date overflow"))?;
        (start, end)
    } else {
        (day, day + Duration::days(1))
    };
    let midnight = |d: chrono::NaiveDate| {
        tz.from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .map(|t| t.timestamp())
            .ok_or_else(|| {
                error(
                    "unsupported_usage_query",
                    "Timezone midnight does not exist",
                )
            })
    };
    Ok((midnight(start)?, midnight(end)?))
}
use chrono::Offset;
fn peak(days: BTreeMap<String, u64>) -> Value {
    let mut best: Option<(String, u64)> = None;
    for (d, n) in days {
        if best.as_ref().is_none_or(|(_, v)| n > *v) {
            best = Some((d, n));
        }
    }
    best.map(|(d, n)| json!({"date":d,"totalTokens":n,"completeness":"partial"}))
        .unwrap_or(Value::Null)
}
fn page_result(
    mut result: Value,
    id: &str,
    offset: usize,
    limit: usize,
) -> Result<UsageQueryResult, ProtocolError> {
    let rows = result["rows"]
        .as_array()
        .ok_or_else(|| db_error("Invalid snapshot"))?;
    if offset > rows.len() {
        return Err(error("invalid_cursor", "Offset out of range"));
    }
    let end = offset.saturating_add(limit).min(rows.len());
    let next = if end < rows.len() {
        Some(format!("{id}:{end}"))
    } else {
        None
    };
    result["rows"] = json!(&rows[offset..end]);
    result["nextCursor"] = json!(next);
    let mut typed: UsageQueryResult = serde_json::from_value(result.clone()).map_err(db_error)?;
    // Serde's nested Option collapses explicit null into absence on deserialization.
    // Preserve requested empty peaks and the unknown-model group on the wire.
    if result["summaries"].get("peakDaily") == Some(&Value::Null) {
        if let Some(summary) = typed.summaries.as_mut() {
            summary.peak_daily = Some(None);
        }
    }
    for (row, raw) in typed
        .rows
        .iter_mut()
        .zip(result["rows"].as_array().unwrap())
    {
        if raw.get("modelId") == Some(&Value::Null) {
            row.model_id = Some(None);
        }
    }
    Ok(typed)
}

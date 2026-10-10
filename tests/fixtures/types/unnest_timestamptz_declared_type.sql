-- A `timestamptz` has no distinct *value* in pglet — it is stored as a UTC `timestamp`,
-- and only the column `TypeOid` carries the tz/no-tz bit. So a FROM-clause `unnest(…)`'s
-- column type must be derived from the **array expression's** static element type, never
-- from the runtime `Value::Array`'s, which has already dropped that bit. pglet's
-- executor derived it from the value and declared `timestamp` (1114) on the simple-query
-- `RowDescription` while `Describe` — which reads the planner's `infer_type` over the
-- same array expression — said `timestamptz` (1184). 18.6 says 1184 on both legs.
--
-- The declared type is not cosmetic here: the text encoder is column-type-aware
-- (`value_to_text_with_oid`), so the wrong declaration also drops the UTC `+00` offset
-- from the rendered value. That half is pinned by `types/unnest_timestamptz_text.sql`.
--
-- `ts` is the polarity pin: a plain `timestamp` element must stay 1114, so a fix that
-- promoted every temporal unnest column to `timestamptz` reddens here. The two arrays
-- also pin that the derivation is **per argument** rather than per unnest node.
-- @test
SELECT tz, ts FROM unnest(
  ARRAY['2026-01-01 12:30:45+00'::timestamptz],
  ARRAY['2026-01-01 12:30:45'::timestamp]
) AS u(tz, ts);

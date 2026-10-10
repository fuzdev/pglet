-- The `::` cast target reads the same type-name table the column path does, so `json` is
-- `42704` there too. The literal is the one that shows why the spelling is refused rather
-- than mapped: 18.6 answers it back byte for byte, duplicate key and all, where a `jsonb`
-- value is deduplicated, reordered and respaced by construction. (The recorded shape is the harness's
-- `serde_json` reparse of that answer, not the answer: the duplicate is gone, and what is
-- left is byte-identical to what `::jsonb` answers on **both** engines, so nothing in this
-- file distinguishes them. The verbatim answer is written down in
-- `crates/pglet/tests/json_type_refused.rs`'s module doc; a fixture that recorded it
-- would have to spell the cast `('…'::json)::text`, so the recorder sees a text column
-- rather than a `json` one and copies the string.)
--
-- **Documentary, not discriminating** — 18.6 accepts, so any pglet error passes here;
-- `crates/pglet/tests/json_type_refused.rs` pins the SQLSTATE.
-- @setup

-- @test
-- @expect-error: type "json" is not supported
SELECT '{"b":1,  "a":2, "b":3}'::json;

-- A duplicate object key resolves to its LAST value, matching Postgres's
-- `jsonb_in` (last-write-wins): `{"a":1,"a":2}` is `{"a": 2}`. Before the dedup
-- fix pglet kept both entries and the first-match extraction paths (`->`, `->>`,
-- `@>`, `=`, `#>>`) silently read the WRONG (first) value. The rendered object
-- uses `a`/`b` keys (a<b) so pglet's first-occurrence insertion order matches
-- Postgres's normalized key order — the ordering divergence is out of scope here.
-- @test
SELECT
  ('{"a":1,"a":2}'::jsonb) ->> 'a'             AS arrow_text,
  ('{"a":1,"a":2}'::jsonb) -> 'a'              AS arrow_json,
  ('{"a":1,"a":2}'::jsonb) @> '{"a":2}'::jsonb AS contains,
  ('{"a":1,"a":2}'::jsonb) = '{"a":2}'::jsonb  AS equals,
  ('{"a":1,"a":2}'::jsonb) #>> '{a}'           AS path_text,
  '{"a":1,"b":2,"a":3}'::jsonb                 AS deduped_obj;

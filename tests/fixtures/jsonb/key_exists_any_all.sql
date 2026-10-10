-- `?|` (any key exists) / `?&` (all keys exist) over a text[]. A NULL array element
-- is SKIPPED (not propagated to NULL), and the empty-array edge is `?|` → false,
-- `?&` → true.
-- @test
SELECT
  '{"a":1,"b":2}'::jsonb ?| ARRAY['x','a']  AS any_hit,        -- t: 'a' present
  '{"a":1,"b":2}'::jsonb ?| ARRAY['x','y']  AS any_miss,       -- f: none present
  '{"a":1,"b":2}'::jsonb ?& ARRAY['a','b']  AS all_hit,        -- t: both present
  '{"a":1,"b":2}'::jsonb ?& ARRAY['a','z']  AS all_miss,       -- f: 'z' absent
  '{"a":1}'::jsonb ?| ARRAY['a', NULL]      AS any_skip_null,  -- t: NULL skipped, 'a' present
  '{"a":1}'::jsonb ?& ARRAY['a', NULL]      AS all_skip_null,  -- t: NULL skipped, 'a' present
  '{"a":1}'::jsonb ?& ARRAY['z', NULL]      AS all_skip_null_miss,  -- f: NULL skipped, 'z' absent
  '{"a":1}'::jsonb ?| ARRAY[]::text[]       AS any_empty,      -- f: empty array
  '{"a":1}'::jsonb ?& ARRAY[]::text[]       AS all_empty       -- t: empty array
;

-- `#>>` — extract at a text[] path AS TEXT. Same traversal as `#>` (object/array/
-- nested/negative index, out-of-range and scalar-mid-path → NULL, empty path → the
-- whole document, a NULL path element → NULL), but the value renders via the `->>`
-- text rule: an array String element comes back unquoted, and a JSON-null target
-- becomes SQL NULL (unlike `#>`, which keeps it as jsonb `null`).
-- @test
SELECT
  '{"a":1,"b":2}'::jsonb #>> '{a}'                 AS obj,           -- 1
  '["x","y","z"]'::jsonb #>> '{1}'                 AS arr,           -- y (unquoted)
  '{"a":{"b":[10,20,30]}}'::jsonb #>> '{a,b,2}'    AS nested,        -- 30
  '{"a":{"b":[10,20,30]}}'::jsonb #>> '{a,b,-1}'   AS neg_index,     -- 30 (last)
  '["x","y"]'::jsonb #>> '{5}'                     AS out_of_range,  -- NULL
  '{"a":5}'::jsonb #>> '{a,b}'                     AS scalar_mid,    -- NULL
  '{"a":1,"b":2}'::jsonb #>> '{}'                  AS whole_doc,     -- {"a": 1, "b": 2}
  ('{"a":1}'::jsonb #>> ARRAY['a', NULL]) IS NULL  AS null_path_elem,        -- t
  ('{"a":null}'::jsonb #>> '{a}') IS NULL          AS json_null_is_sql_null  -- t
;

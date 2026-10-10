-- `#>` — extract jsonb at a text[] path. Object key, array index (a negative index
-- counts from the end), out-of-range and scalar-mid-path both yield NULL (no
-- `?`-style scalar quirk), the empty path returns the whole document, a NULL path
-- element makes the whole result NULL, and a JSON-null target stays jsonb `null`
-- (not SQL NULL — the `#>>` fixture covers that distinction). Object literals use
-- keys already in sorted order so the jsonb text rendering is deterministic.
-- @test
SELECT
  '{"a":1,"b":2}'::jsonb #> '{a}'                AS obj,           -- 1
  '["x","y","z"]'::jsonb #> '{1}'                AS arr,           -- "y"
  '{"a":{"b":[10,20,30]}}'::jsonb #> '{a,b,2}'   AS nested,        -- 30
  '{"a":{"b":[10,20,30]}}'::jsonb #> '{a,b,-1}'  AS neg_index,     -- 30 (last)
  '["x","y"]'::jsonb #> '{5}'                    AS out_of_range,  -- SQL NULL
  '{"a":5}'::jsonb #> '{a,b}'                    AS scalar_mid,    -- SQL NULL
  '{"a":1,"b":2}'::jsonb #> '{}'                 AS whole_doc,     -- {"a": 1, "b": 2}
  ('{"a":1}'::jsonb #> ARRAY['a', NULL]) IS NULL AS null_path_elem,  -- t
  '{"a":null}'::jsonb #> '{a}'                   AS json_null     -- null (jsonb, not SQL NULL)
;

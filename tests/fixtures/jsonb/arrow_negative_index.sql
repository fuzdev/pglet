-- `->` array indexing counts a negative index from the end, exactly as `#>` does
-- (both now share `resolve_array_index`). pglet used to cast the index straight to
-- `usize`, so a negative index wrapped to a huge value and always missed.
-- Shape mismatches stay NULL rather than erroring: a string key against an array
-- and an integer index against an object are both SQL NULL in Postgres.
-- @test
SELECT
  '[1,2,3]'::jsonb -> -1                     AS last,           -- 3
  '[1,2,3]'::jsonb -> -3                     AS first,          -- 1
  ('[1,2,3]'::jsonb -> -4) IS NULL           AS under_range,    -- t
  ('[1,2,3]'::jsonb -> 3) IS NULL            AS over_range,     -- t
  '["x","y","z"]'::jsonb ->> -2              AS last_but_one,   -- y
  ('[1,2,3]'::jsonb -> 'a') IS NULL          AS key_on_array,   -- t
  ('{"a":1}'::jsonb -> 0) IS NULL            AS index_on_object -- t
;

-- jsonb B-tree tie-breaks that raw JSON text order gets wrong: arrays compare by
-- length first, objects by pair count first, and cross-type by rank (a number sorts
-- AFTER a string).
-- @test
SELECT
  ('[1,2]'::jsonb   < '[1,2,0]'::jsonb)       AS arr_len,     -- length first
  ('[1]'::jsonb     < '[0,0]'::jsonb)         AS arr_len2,    -- shorter wins
  ('[1,2]'::jsonb   < '[1,3]'::jsonb)         AS arr_elem,    -- same len, element
  ('{"a":1}'::jsonb < '{"a":1,"b":1}'::jsonb) AS obj_pairs,   -- pair count first
  ('{"a":2}'::jsonb < '{"a":3}'::jsonb)       AS obj_val,     -- key eq, value
  ('1'::jsonb       < '"z"'::jsonb)           AS num_vs_str   -- number > string
;

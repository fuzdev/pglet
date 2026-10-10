-- `?` — jsonb key/element existence. An object top-level key, a String array
-- element, and Postgres's top-level-scalar-String quirk (`'"a"'::jsonb ? 'a'` is
-- true) all match; a non-string scalar never does; a NULL operand yields NULL.
-- @test
SELECT
  '{"a":1,"b":2}'::jsonb ? 'a'   AS obj_key,       -- t: object key present
  '{"a":1}'::jsonb ? 'x'         AS obj_miss,      -- f: object key absent
  '["a","b","c"]'::jsonb ? 'b'   AS arr_elem,      -- t: String array element
  '["a","b"]'::jsonb ? 'z'       AS arr_miss,      -- f: no such element
  '"a"'::jsonb ? 'a'             AS scalar_quirk,  -- t: scalar String == key
  '1'::jsonb ? '1'               AS num_scalar,    -- f: non-string scalar never matches
  (NULL::jsonb ? 'a') IS NULL    AS null_operand   -- t: NULL operand → NULL
;

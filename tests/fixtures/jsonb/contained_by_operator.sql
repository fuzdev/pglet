-- `<@` is `@>` with the operands swapped, so every widened rule reaches it too —
-- worth locking separately because the swap happens at a different layer (the
-- planner hands `containment()` the operands already exchanged) and a fix applied
-- to only one direction is a plausible way to get this half wrong.
-- @setup
CREATE TABLE jcby (id INT, d JSONB);
INSERT INTO jcby VALUES (1, '[1,2]'), (2, '{"a":1}'), (3, '"x"');
-- @test
SELECT
  ('[1,3]'::jsonb <@ '[1,2,3]'::jsonb) AS array_subset,
  ('{"tags":[1]}'::jsonb <@ '{"tags":[1,2,3]}'::jsonb) AS nested_array,
  ('"a"'::jsonb <@ '["a","b"]'::jsonb) AS raw_scalar,
  ('["a"]'::jsonb <@ '"a"'::jsonb) AS array_not_in_scalar,
  ('{"a":1,"b":2}'::jsonb <@ '{"a":1}'::jsonb) AS not_contained,
  (SELECT count(*) FROM jcby WHERE d <@ '[1,2,3]'::jsonb) AS from_column;

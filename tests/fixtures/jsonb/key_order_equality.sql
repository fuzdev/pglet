-- Object key order is **not semantically significant** in jsonb: `jsonb_in`
-- canonicalizes on input, so two objects with the same pairs are the same value
-- however they were spelled.
--
-- pglet stored objects in input order and compared them pairwise in that order, so
-- `'{"a":1,"b":2}' = '{"b":2,"a":1}'` was **false** — a wrong row identity, not a
-- wrong rendering, which is why it reached `=`, `DISTINCT`, `GROUP BY`, and join
-- dedup alike. The column comparisons here run the values through the storage
-- round-trip rather than comparing two literals in the planner.
-- @setup
CREATE TABLE jkoe (id INT, d JSONB);
INSERT INTO jkoe VALUES
  (1, '{"a":1,"b":2}'),
  (2, '{"b":2,"a":1}'),
  (3, '{"a":1,"b":3}');
-- @test
SELECT
  ('{"a":1,"b":2}'::jsonb = '{"b":2,"a":1}'::jsonb) AS literal_equality,
  ('{"a":1,"b":2}'::jsonb <> '{"a":1,"b":3}'::jsonb) AS still_distinguishes_values,
  (SELECT count(*) FROM jkoe WHERE d = '{"b":2,"a":1}'::jsonb) AS matching_rows,
  (SELECT count(*) FROM jkoe a JOIN jkoe b ON a.d = b.d WHERE a.id < b.id) AS join_pairs,
  ('{"x":{"a":1,"b":2}}'::jsonb = '{"x":{"b":2,"a":1}}'::jsonb) AS nested_equality;

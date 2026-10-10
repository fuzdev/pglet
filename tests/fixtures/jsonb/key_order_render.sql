-- Canonical key order is visible in the **rendering**, and it is not lexicographic:
-- Postgres stores object keys by `lengthCompareJsonbString` — length first, then
-- bytewise — so `{"b":1,"aa":2,"c":3}` comes back as `{"b": 1, "c": 3, "aa": 2}`.
--
-- The short-key-before-bytewise-smaller-long-key case is the one a plain
-- lexicographic sort gets wrong, so it is the case worth locking. pglet rendered
-- input order.
-- @setup
CREATE TABLE jkor (id INT, d JSONB);
INSERT INTO jkor VALUES (1, '{"b":1,"aa":2,"c":3}');
-- @test
SELECT
  '{"b":1,"aa":2,"c":3}'::jsonb::text AS length_then_bytes,
  '{"zz":1,"a":2}'::jsonb::text AS short_key_first,
  '{"z":{"bb":1,"a":2}}'::jsonb::text AS nested_canonical,
  '{"a":1,"a":2}'::jsonb::text AS duplicate_last_wins,
  (SELECT d::text FROM jkor WHERE id = 1) AS through_storage;

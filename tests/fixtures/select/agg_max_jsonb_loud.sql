-- Same as the bool case on the other un-registered type: `jsonb` orders fully
-- (the `jsonb/order_*` fixtures cover that), but Postgres has no `max(jsonb)`.
-- @setup
CREATE TABLE mj (doc JSONB);
INSERT INTO mj VALUES ('{"a":1}'), ('{"a":2}');
-- @test
-- @expect-error: no max(jsonb)
SELECT MAX(doc) FROM mj;

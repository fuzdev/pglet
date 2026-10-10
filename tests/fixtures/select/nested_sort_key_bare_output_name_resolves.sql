-- @setup
CREATE TABLE nskb (q INT, a INT, b INT);
INSERT INTO nskb VALUES (1, 1, 10), (2, 2, 20), (3, 2, 30);

-- The **accept** polarity of `nested_sort_key_nested_output_name_loud.sql` and its
-- aggregate sibling: the same two keys written *bare*, which is the shape
-- `findTargetlistEntrySQL92` does reach. A fix that gated the re-walk's output names on
-- anything wider than the key's own shape breaks these; both were `42703` here before the
-- output names reached a nested level at all.
-- @test
SELECT (SELECT q * 10 AS z FROM nskb ORDER BY z LIMIT 1) AS by_alias,
       (SELECT count(*) AS c FROM nskb GROUP BY a ORDER BY c LIMIT 1) AS by_agg_alias;

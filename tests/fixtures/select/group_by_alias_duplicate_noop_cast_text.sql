-- The `text` leg of `group_by_alias_duplicate_noop_cast.sql`: the no-op-cast strip is
-- driven by the column's type from the FROM scope, not by a hardcoded numeric family,
-- so `s::text` over a `text` column elides exactly as `x::integer` over an `integer`
-- one does.
-- @setup
CREATE TABLE gbdnct (s TEXT);
INSERT INTO gbdnct VALUES ('p'), ('q'), ('q');
-- @test
SELECT count(*) AS n FROM (SELECT s AS a, s::text AS a FROM gbdnct GROUP BY a) q;

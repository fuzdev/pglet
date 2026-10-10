-- A sort key that is an expression over the `$N` rather than the `$N` itself equals no
-- entry, so the parameter stays unknown and takes the other side's `integer`.
-- @setup
CREATE TABLE upo (n INT);
INSERT INTO upo VALUES (1);
-- @param $1 int4 5
-- @test
SELECT 1 AS v UNION ALL (SELECT $1 FROM upo ORDER BY $1 + 0) ORDER BY 1;

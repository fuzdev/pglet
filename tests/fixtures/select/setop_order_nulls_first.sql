-- A set-op (`UNION`) trailing `ORDER BY 1 NULLS FIRST` — the positional key carries
-- the explicit NULL placement, binding to the whole combined result. NULLs sort
-- first, differing from the ASC positional default (NULLS LAST). Guards that the
-- placement survives the set-op ORDER BY path.
-- @setup
CREATE TABLE a (v INT);
CREATE TABLE b (v INT);
INSERT INTO a VALUES (10), (NULL), (30);
INSERT INTO b VALUES (20), (NULL), (10);
-- @test
SELECT v FROM a UNION SELECT v FROM b ORDER BY 1 NULLS FIRST;

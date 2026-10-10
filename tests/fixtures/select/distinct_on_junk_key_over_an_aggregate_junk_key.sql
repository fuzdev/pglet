-- @setup
CREATE TABLE dtm (k INT, v INT);
INSERT INTO dtm VALUES (1, 10), (1, 10), (1, 20), (2, 30);

-- **Both minters on one statement**, which is the only shape the trim-width override is
-- load-bearing for. The `DISTINCT ON (k)` key mints the first junk column on the
-- Aggregate; the trailing `ORDER BY v` key — a grouping column the select list also
-- drops — mints the second, from `build_sort`, which reports the Aggregate's *current*
-- expression count (2) as the width to keep. The query's real output is one column, so
-- the trim has to be the select list's width and not either minter's: taking
-- `SortedPlan::junk_keep` here emits `pglet_junk_0` beside `c`.
-- @test
SELECT DISTINCT ON (k) count(*) AS c FROM dtm GROUP BY k, v ORDER BY k, v;

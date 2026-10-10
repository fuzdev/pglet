-- The dropped-key half of `distinct_on_compound_key_reads_the_projected_column.sql`:
-- the projection carries the ON key's base column under **no** name, so there is
-- nothing above the projection for `Plan::DistinctOn` to read. The key rides the
-- projecting node as a junk column instead, the way Postgres's
-- `transformDistinctOnClause` reaches `addTargetToSortList`.
--
-- The **compound** spelling is the discriminating one. It is not an `Expr::Column`,
-- so the whole-key test never saw it: before the walk reached inside the key it read
-- the dropped column off the projected row by bare name, silently — and once the sort
-- below correctly ordered by the base column while the dedup grouped by a different
-- projected one, the answer was `5 7 5`, not a legal `DISTINCT ON` result of any
-- input. 18.6 answers `5 5`.
-- @setup
CREATE TABLE dcd (a INT, b INT);
INSERT INTO dcd VALUES (1, 5), (1, 7), (2, 5);

-- @test
SELECT DISTINCT ON (a+0) b AS c FROM dcd ORDER BY a+0, b;

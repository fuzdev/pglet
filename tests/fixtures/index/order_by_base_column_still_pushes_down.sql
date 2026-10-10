-- The accept half of `order_by_output_alias_declines_pushdown.sql`: the projection
-- carries the indexed base column under its own name, so the whole bare key `a` binds
-- to an output column that *denotes that very base column* and the index walk is
-- order-correct. The pair is what says the gate tests the *binding* rather than the
-- presence of a matching output name.
--
-- **It pins the answer, and cannot pin the plan.** Declining the pushdown only keeps
-- the `Sort`, which sorts the same rows into the same order — so this fixture is green
-- whether or not the optimization fires, and an over-declining gate is invisible to the
-- whole differential corpus. That happened: a first version read the projection list
-- literally and disabled the pushdown for every `SELECT *`, and every fixture stayed
-- green while seven `planner::tests` plan-shape tests went red. Those are what hold it;
-- this holds only that the rows are right.
-- @profile-lock
-- @setup
CREATE TABLE obp (a INT NOT NULL, b INT NOT NULL);
CREATE INDEX obp_a ON obp(a);
INSERT INTO obp VALUES (2, 20), (1, 10);

-- @test
SELECT a FROM obp ORDER BY a;

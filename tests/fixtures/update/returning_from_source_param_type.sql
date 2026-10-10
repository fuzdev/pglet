-- A bound `$N` compared with an `UPDATE … FROM` **source** column, so the server's
-- inferred parameter type is what decides whether the bind succeeds at all.
--
-- The clause's parameter-type walk resolved a `RETURNING` column against the catalog
-- keyed by its qualifier, falling back to the target — and an alias names no catalog
-- table, so `s.b` took the *target's* `b`. `rpb_t.b` is `bigint` and `rpb_s.b` is
-- `integer`, so the reported OID was `int8` where 18.6 says `int4`.
--
-- That is visible to this harness, which the previous pass's write-up said it would not
-- be. `-- @param` supplies the value's *Rust* type, not the wire OID, and
-- tokio-postgres binds it against the OID the server reported in
-- `ParameterDescription` (`postgres_types::__to_sql_checked` refuses when
-- `T::accepts(ty)` is false) — so an `int4` value against an `int8` report never
-- reaches the statement. Measured: this file is red with the clause scope removed and
-- green with it, as is its `…_only_param_type` twin for the OID-`0` shape.
--
-- What a fixture still cannot see is a **wrong OID the bound value happens to accept**
-- — two names for one Rust type, or a report that is merely imprecise. For those the
-- guard has to be `planner::param_types`'s unit tests, which read the inferred OID
-- directly.
-- @setup
CREATE TABLE rpb_t (id INT, b BIGINT);
CREATE TABLE rpb_s (id INT, b INT);
INSERT INTO rpb_t VALUES (1, 111);
INSERT INTO rpb_s VALUES (1, 5);

-- @test
-- @param $1 int4 5
UPDATE rpb_t SET id = rpb_t.id FROM rpb_s s WHERE s.id = rpb_t.id RETURNING (s.b = $1) AS eq;

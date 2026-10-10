-- The same missing identity as `update/correlated_outer_ref_unaliased_target_loud.sql`,
-- reached through the **other** resolver: no subquery, no quoted dotted column, just a
-- qualifier naming no relation in the statement. A `SimpleColumns` with no relation
-- identity drops the qualifier and matches the bare name, so this resolved `ex1`'s own
-- `n` and **wrote the row**; 18.6 raises `42P01`.
--
-- Two fixtures because two resolvers read the identity and either could regress alone:
-- this one pins `SimpleColumns::resolve`'s last tier, that one pins
-- `EvalEnv::resolve_outer_slot`'s dotted tier. The same statement spelled `DELETE FROM
-- ex1 WHERE bogusrel.n = 1`, or with the target aliased, was `42P01` on both engines
-- throughout — `execute_delete` always passed a name, and an aliased UPDATE always had
-- one.
--
-- The SET right-hand side takes the same route and the same fix: `UPDATE ex1 SET v =
-- bogusrel.n + 100` read `ex1`'s own `n` and stored `101`, and is `42P01` on both now.
-- It is not spelled here because one `@test` is one statement; the pair is asserted
-- together, with the rows checked, in `crates/pglet`'s `scope_relation_identity.rs`.
-- @setup
CREATE TABLE ex1 (n INT, v INT);
INSERT INTO ex1 VALUES (1, 1);

-- @expect-error: missing FROM-clause entry for table "bogusrel" (42P01)
-- @test
UPDATE ex1 SET v = 42 WHERE bogusrel.n = 1;

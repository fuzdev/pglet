-- A **correlated scalar subquery** in `RETURNING`, correlating on a `FROM` **source**
-- column.
--
-- The spine's own shape (`RETURNING *, (SELECT COUNT(*) … WHERE cell_id = cell.id)`)
-- correlates on the *target*; this is the same mechanism reaching the other half of the
-- namespace. The subquery's outer scope is the projected row, so widening that row is
-- what makes `s.k` visible inside it — a build that kept only the target answers `42703`
-- rather than a wrong number.
--
-- `rcs_x` holds a row keyed `7`, so a subquery bound to the source's `k` counts 1 and one
-- bound to the target's `id` counts 0 — the two relations are chosen so a mis-binding is
-- a different number rather than the same one.
--
-- **Divergence, pre-existing and not this fixture's subject:** the *bare* spelling of the
-- same reference resolves on 18.6 and is `42703` here. The executor addresses a
-- multi-relation outer scope by qualified name only, which is the same answer a plain
-- `SELECT` gives over a join — measured on both, with an inner relation that does not
-- carry the name: `SELECT (SELECT count(*) FROM y WHERE y.yk = k) FROM t JOIN s ON …` is
-- `42703` here and answers there. Loud, so it is a gap and not a wrong answer; recorded
-- in `EDGES.md`.
-- @setup
CREATE TABLE rcs_t (id INT, v TEXT);
CREATE TABLE rcs_s (id INT, k INT);
CREATE TABLE rcs_x (k INT);
INSERT INTO rcs_t VALUES (1, 'TARGET');
INSERT INTO rcs_s VALUES (1, 7);
INSERT INTO rcs_x VALUES (7);

-- @test
UPDATE rcs_t SET v = 'hit' FROM rcs_s s WHERE s.id = rcs_t.id
  RETURNING (SELECT count(*) FROM rcs_x WHERE rcs_x.k = s.k) AS c;

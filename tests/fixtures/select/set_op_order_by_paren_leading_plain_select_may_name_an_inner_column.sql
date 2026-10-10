-- Why the set-op key gate cannot close over every caller of `plan_set_op_order_limit`.
-- The same parser arm serves a parenthesized **plain** select, whose sort clause 18.6
-- folds into the inner query — so its key may name an inner FROM column the projection
-- drops, and `b` is one. Gating this spelling would refuse a statement both engines
-- answer, with a `42703` naming a column that exists.
--
-- The CTE is unreferenced deliberately: referencing it reaches a *separate*, pre-existing
-- divergence (a paren-leading plain select's key is judged against its output columns
-- alone here, recorded in `EDGES.md` row 167), which would make this fixture red for a reason that
-- is not its subject.
-- @setup
CREATE TABLE sopt (a INT, b INT);
INSERT INTO sopt VALUES (1, 9), (2, 8);
CREATE TABLE sopn (a INT);
INSERT INTO sopn VALUES (1), (2);

-- @test
WITH c AS ((SELECT a FROM sopt) ORDER BY b) SELECT count(*) AS n FROM sopn;

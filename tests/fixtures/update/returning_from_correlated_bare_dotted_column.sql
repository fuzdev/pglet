-- The `UPDATE … FROM` `RETURNING` clause resolves by `(relation, column)` pair, and its
-- correlated **outer frame** now reads the same pairs. It used to be handed the flat
-- synthesized `relation.column` names instead, so a *bare* reference inside a `RETURNING`
-- subquery — the one path no qualifier can gate — matched the target's own `v` under its
-- synthesized spelling rather than the source's quoted column, and the count came back
-- `0` for both rows. Silent, both engines accepting.
--
-- Two controls sit beside it in this file's shape and agree on both engines throughout:
-- the qualified spelling inside the subquery (`s."rfc_w.v"`), and the same reference at
-- the clause's top level, which the pair-keyed clause already resolved correctly.
-- @setup
CREATE TABLE rfc_w (id INT, v TEXT);
CREATE TABLE rfc_s (id INT, "rfc_w.v" TEXT);
CREATE TABLE rfc_o (yv TEXT);
INSERT INTO rfc_w VALUES (1, 'a'), (2, 'b');
INSERT INTO rfc_s VALUES (1, 'srcval'), (2, 'srcval2');
INSERT INTO rfc_o VALUES ('srcval');

-- @test
UPDATE rfc_w SET v = rfc_w.v FROM rfc_s s WHERE s.id = rfc_w.id
RETURNING (SELECT count(*) FROM rfc_o WHERE rfc_o.yv = "rfc_w.v") AS n, rfc_w.id;

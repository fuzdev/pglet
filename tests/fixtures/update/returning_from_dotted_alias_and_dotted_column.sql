-- Two **qualified** `RETURNING` references whose `relation.column` spellings are the
-- same string: the target `rda_t`'s quoted column `"x.y"` and the column `y` of a source
-- aliased `"rda_t.x"`. Both flatten to `rda_t.x.y`.
--
-- It exists to reject a fix that keeps the flat string namespace and merely stops bare
-- names reaching it — the plausible shape being "bind each bare leaf to its relation at
-- plan time and let the executor resolve only qualified references". That closes both
-- headline fixtures and this one still splits, because the collision is in the
-- *flattening*, not in the spelling of the reference: the first match wins and one of
-- the two columns answers for the other.
--
-- A dotted **alias** is what makes the pair reachable, and both engines accept it. The
-- two values differ (`TARGETXY` against `SOURCEY`) so a mis-binding shows as a value and
-- not only as a type.
-- @setup
CREATE TABLE rda_t (id INT, "x.y" TEXT);
CREATE TABLE rda_s (id INT, y TEXT);
INSERT INTO rda_t VALUES (1, 'TARGETXY');
INSERT INTO rda_s VALUES (1, 'SOURCEY');

-- @test
UPDATE rda_t SET id = rda_t.id FROM rda_s AS "rda_t.x" WHERE "rda_t.x".id = rda_t.id
  RETURNING rda_t."x.y" AS tgt, "rda_t.x".y AS src;

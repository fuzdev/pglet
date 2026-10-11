-- **A false-rejection control — do not trim it**, and the fixture that says a
-- sort-below-projection layout still claims **positions**. `ORDER BY 2` names the
-- second entry, so the first stays Postgres's `unknown` and assigns into a `BOOL`
-- column; the `ORDER BY 1, n` spelling next door is `42804`. Accepted on 18.6.
--
-- It also keeps the layout's own rewrite correct. Sort-below carries a naming key down
-- as the projection expression that produced it, and here that expression is the bare
-- integer `5` — which is *also* the plan's spelling of a positional reference. Carried
-- raw, the Sort read base column 5 of a one-column source and this statement was
-- `42P10 ORDER BY position 5 is not in select list`, on a statement both engines accept.
-- @setup
CREATE TABLE acsp (id INT, b BOOL, i INT);
CREATE TABLE acspsrc (n INT);
INSERT INTO acspsrc VALUES (1), (2);
-- @test
INSERT INTO acsp (b, i) SELECT 't', 5 FROM acspsrc ORDER BY 2, n RETURNING b, i;

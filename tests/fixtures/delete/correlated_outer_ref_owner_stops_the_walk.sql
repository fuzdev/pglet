-- **An owning frame stops the walk, resolved or not** — Postgres's `colNameToVar` never
-- looks past the level that claims a qualifier. Here the outer frame *is* `ows_a` and
-- has no `v`, while the intervening frame `b` has one, so the reference used to rebind
-- to `b`'s column and this statement was `DELETE 2` — every row gone — where 18.6
-- answers `42703 column ows_a.v does not exist` (hinting `b.v`) with both rows intact.
--
-- It is the shape no single-frame test reaches: with only one outer frame the reference
-- simply found nothing and the statement was already loud (wrong code, but loud). The
-- destruction needs a *second* frame carrying the name, which is why the fixture nests
-- two levels of `EXISTS`.
--
-- The `42703` is Postgres's, not a stand-in: the qualifier resolves, so the missing thing
-- is the column. Controls on both engines: the ordinary `ows_k.k = b.v` answers, and
-- `zzz.v` — a qualifier no level owns — is `42P01`, the neighbouring arm.
-- @setup
CREATE TABLE ows_a (id INT);
CREATE TABLE ows_b (v INT);
CREATE TABLE ows_k (k INT);
INSERT INTO ows_a VALUES (1), (2);
INSERT INTO ows_b VALUES (7);
INSERT INTO ows_k VALUES (7);

-- @expect-error: column ows_a.v does not exist (42703)
-- @test
DELETE FROM ows_a WHERE EXISTS (SELECT 1 FROM ows_b b WHERE EXISTS (SELECT 1 FROM ows_k WHERE ows_k.k = ows_a.v));

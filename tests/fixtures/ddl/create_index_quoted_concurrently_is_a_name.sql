-- `CONCURRENTLY` is a contextual identifier, matched only **unquoted**, so a
-- `"concurrently"`-quoted word is an ordinary index name on both engines — the
-- keyword and the name live in different spaces. `@no-txn` so the create is a real
-- one rather than the `25001` an unquoted spelling would take.
--
-- **A lock, not a new assertion**: it is green against an engine for which
-- `concurrently` is nothing but an identifier. That is what makes it worth keeping —
-- teaching the parser a new contextual keyword is exactly the change that could make
-- the match stop being quoting-aware, and this is the fixture that would redden.
-- @no-txn
-- @setup
DROP TABLE IF EXISTS tiq;
CREATE TABLE tiq (id INT);
INSERT INTO tiq VALUES (1);
CREATE INDEX "concurrently" ON tiq (id);
DROP INDEX "concurrently";
-- @test
SELECT id FROM tiq;

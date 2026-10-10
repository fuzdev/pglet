-- `UPDATE … FROM src s` where the source and the target share a column name, and the
-- WHERE compares the **source's** column against a bare string literal. The source's
-- relations were never registered for type resolution, so `s.x` either resolved
-- against the *target*'s `x` (text — no cast, leaving `integer = text` for the eval
-- gate) or resolved to nothing at all (same outcome): a `42883` on an UPDATE Postgres
-- performs. Registering the source's relations resolves `s.x` to its own `integer` and
-- the literal is cast to match.
-- @setup
CREATE TABLE ufs_tgt (id INT, x TEXT);
CREATE TABLE ufs_src (id INT, x INT);
INSERT INTO ufs_tgt VALUES (1, 'a'), (2, 'b');
INSERT INTO ufs_src VALUES (1, 5), (2, 6);
-- @test
UPDATE ufs_tgt SET x = 'hit' FROM ufs_src s WHERE s.id = ufs_tgt.id AND s.x = '5' RETURNING ufs_tgt.id, ufs_tgt.x;

-- **A silent wrong answer, closed.** A qualified reference resolves against the relation
-- its qualifier names and no other, so `jpw_a.y` — a column only `jpw_b` has — is
-- `42703` on 18.6. pglet **answered `jpw_b`'s column**: the post-projection resolver
-- carries no relation identity, and its last tier drops the qualifier and takes the one
-- name ending `.y`.
--
-- Two controls, both agreeing on either engine: `jpw_b.y` answers, and `bogusrel.y` — a
-- qualifier naming no relation at all — is `42P01` rather than `42703`, which is the
-- split this gate reproduces (`errorMissingRTE` vs `errorMissingColumn`).
-- @setup
CREATE TABLE jpw_a (id INT, x TEXT);
CREATE TABLE jpw_b (id INT, y TEXT);
INSERT INTO jpw_a VALUES (1, 'ax');
INSERT INTO jpw_b VALUES (1, 'by');

-- @expect-error: column jpw_a.y does not exist (42703)
-- @test
SELECT jpw_a.y FROM jpw_a JOIN jpw_b ON jpw_b.id = jpw_a.id;

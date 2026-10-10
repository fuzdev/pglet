-- A plain column's `nextval` default is still the user's to clear — the shape a restored
-- `SERIAL` carries, since pg_dump re-attaches it with `ALTER COLUMN ... SET DEFAULT`
-- rather than the `SERIAL` keyword. The identity gate must therefore key on the column's
-- identity flag and not on its default *expression*: read the expression and this
-- ordinary statement becomes 42601, silently, on the one path a restore script uses.
-- The refusal side of that same fork is pinned by unit tests, but only a fixture pins the
-- accept side against the oracle.
-- @setup
CREATE SEQUENCE acdnp_seq START WITH 10;
CREATE TABLE acdnp_t (id INT NOT NULL DEFAULT nextval('acdnp_seq'), v INT);
INSERT INTO acdnp_t (v) VALUES (1);
ALTER TABLE acdnp_t ALTER COLUMN id DROP DEFAULT;
INSERT INTO acdnp_t (id, v) VALUES (99, 2);
-- @test
SELECT id, v FROM acdnp_t ORDER BY v;

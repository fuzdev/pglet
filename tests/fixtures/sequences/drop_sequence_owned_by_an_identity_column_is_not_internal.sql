-- Being `OWNED BY` an identity column is not the same fact as *being* that column's
-- sequence. `ALTER SEQUENCE … OWNED BY` records an AUTO edge whatever the column is,
-- so this drop succeeds where the identity's own sequence is 2BP01 — the near-miss a
-- gate reading only the owner column's identity flag would refuse.
-- @setup
CREATE TABLE dsoid_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
CREATE SEQUENCE dsoid_other START WITH 20;
ALTER SEQUENCE dsoid_other OWNED BY dsoid_t.id;
DROP SEQUENCE dsoid_other;
INSERT INTO dsoid_t (b) VALUES (1);
-- @test
SELECT id, b FROM dsoid_t;

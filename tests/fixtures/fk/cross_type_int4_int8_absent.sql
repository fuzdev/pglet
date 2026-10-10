-- Same cross-type INT4 -> INT8 FK, but the referenced key has no parent row: Postgres
-- (and pglet, post-coercion) still rejects with 23503 — coercing the child value to the
-- parent type accepts VALID references without loosening enforcement of absent ones.
-- @profile-lock
-- @setup
CREATE TABLE ctia_parent (id INT8 PRIMARY KEY);
CREATE TABLE ctia_child (fk INT4 REFERENCES ctia_parent(id));
INSERT INTO ctia_parent VALUES (5);

-- @test
-- @expect-error: a cross-type FK to an absent parent still violates
INSERT INTO ctia_child VALUES (7);

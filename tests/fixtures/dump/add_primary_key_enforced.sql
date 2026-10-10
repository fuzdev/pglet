-- pg_dump adds a table's PRIMARY KEY after the data load via ALTER TABLE ONLY
-- ... ADD CONSTRAINT. pglet must build the enforced unique index (not no-op it),
-- so a later duplicate insert is rejected with 23505 — matching Postgres.
-- @profile-lock
-- @setup
CREATE TABLE dpk (id integer NOT NULL, name text);
INSERT INTO dpk VALUES (1, 'a'), (2, 'b');
ALTER TABLE ONLY dpk
    ADD CONSTRAINT dpk_pkey PRIMARY KEY (id);
-- @test
INSERT INTO dpk (id, name) VALUES (1, 'dup');
-- @expect-error: duplicate key violates the PK added by ADD CONSTRAINT

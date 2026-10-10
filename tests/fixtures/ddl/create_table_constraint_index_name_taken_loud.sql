-- A declared constraint name is also an *index* name — Postgres puts both in one
-- relation namespace — so a `CONSTRAINT <n> PRIMARY KEY` whose name an existing index
-- already holds is 42P07 (`relation "<n>" already exists`), even across tables.
-- @setup
CREATE TABLE ctin_other (a INT);
CREATE UNIQUE INDEX ctin_taken ON ctin_other (a);

-- @expect-error: relation ctin_taken already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ctin (a INT NOT NULL, CONSTRAINT ctin_taken PRIMARY KEY (a));

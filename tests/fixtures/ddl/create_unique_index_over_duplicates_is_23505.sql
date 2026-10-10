-- `CREATE [UNIQUE] INDEX` had **no build**: it registered the index and back-filled it
-- without ever asking whether the existing rows already duplicate a key, so a unique
-- index over duplicate rows was created and simply never held. The consequence is a row
-- divergence, not a catalog one — 18.6 creates no index and takes the next duplicate
-- row, pglet refused it `23505` — so the two engines went on holding different data.
--
-- `ALTER TABLE … ADD CONSTRAINT UNIQUE` / `ADD PRIMARY KEY` have always validated
-- (`validate_existing_unique`); this route is the one that did not, and the gate now
-- runs through the **write path's own** key preparation, so what the build refuses and
-- what a later write refuses cannot drift.
--
-- The position is 18.6's: `index_build` runs inside `index_create`, *after* the name
-- conflict and the `IF NOT EXISTS` skip that call also owns — measured, `CREATE UNIQUE
-- INDEX IF NOT EXISTS <a taken name>` over duplicate rows reports success on both
-- engines.
-- @setup
CREATE TABLE cuid_t (a INT, b INT);
INSERT INTO cuid_t VALUES (1, 7), (2, 7);
-- @test
CREATE UNIQUE INDEX cuid_ix ON cuid_t (b);
-- @expect-error: the index build finds the duplicated key (23505)
-- @expect-message
-- @profile-lock

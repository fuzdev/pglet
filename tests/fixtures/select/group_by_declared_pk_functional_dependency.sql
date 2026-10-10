-- A GROUP BY on the primary key makes every other column functionally dependent, so
-- it needs no aggregate. pglet finds the PK by the catalog's constraint marker, not by
-- the `<table>_pkey` name — a declared `CONSTRAINT <n> PRIMARY KEY` would otherwise
-- look like "no primary key" and raise a spurious 42803.
-- @setup
CREATE TABLE gbd (id INT NOT NULL, name TEXT, CONSTRAINT gbd_my_pk PRIMARY KEY (id));
INSERT INTO gbd VALUES (1, 'a'), (2, 'b');

-- @profile-lock
-- @test
SELECT id, name FROM gbd GROUP BY id ORDER BY id;

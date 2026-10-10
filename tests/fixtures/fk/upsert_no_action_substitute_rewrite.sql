-- A later `DO UPDATE` rewrite that moves another row onto the vacated key satisfies
-- `NO ACTION` as well.
-- @profile-lock
-- @setup
CREATE TABLE upnr_p (id INT PRIMARY KEY, v INT);
CREATE TABLE upnr_c (pid INT REFERENCES upnr_p(id), x INT);
INSERT INTO upnr_p VALUES (1, 1), (2, 2);
INSERT INTO upnr_c VALUES (1, 10);
INSERT INTO upnr_p VALUES (1, 5), (2, 6) ON CONFLICT (id) DO UPDATE SET id = CASE upnr_p.id WHEN 1 THEN 5 ELSE 1 END;

-- @test
SELECT id, v FROM upnr_p ORDER BY id;

-- The action converts the parent's new key once, ahead of looking for the rows it rewrites,
-- so the conversion refuses though no child row references the moved key. 18.6 answers the
-- same while the action's plan is a custom plan, which folds the conversion over the bound
-- key when the action is planned — each of a session's first five firings of one key's
-- action (`EDGES.md` row 200 is the later firings).
-- @setup
CREATE TABLE p (k varchar(10) PRIMARY KEY);
CREATE TABLE c (id int PRIMARY KEY, pk varchar(3) REFERENCES p ON UPDATE CASCADE);
INSERT INTO p VALUES ('a'), ('b');
INSERT INTO c VALUES (1, 'b');
-- @test
-- @profile-lock
-- @expect-error: value too long for type character varying(3)
UPDATE p SET k = 'abcdef' WHERE k = 'a';

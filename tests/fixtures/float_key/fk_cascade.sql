-- Cascading actions find their float children: an update repoints the child holding
-- `-0`, a delete removes the NaN child, and a `numeric` child of a `real` parent follows
-- its parent's new key.
-- @profile-lock
-- @setup
CREATE TABLE fkpc (x FLOAT8 PRIMARY KEY);
CREATE TABLE fkcc (id INT, x FLOAT8 REFERENCES fkpc (x) ON DELETE CASCADE ON UPDATE CASCADE);
INSERT INTO fkpc VALUES (0), ('NaN'), (1.5);
INSERT INTO fkcc VALUES (1, '-0'), (2, 'NaN'), (3, 1.5);
UPDATE fkpc SET x = 7 WHERE x = 0;
DELETE FROM fkpc WHERE x = 'NaN';
CREATE TABLE fkpr (x REAL PRIMARY KEY);
CREATE TABLE fkcr (id INT, x NUMERIC REFERENCES fkpr (x) ON UPDATE CASCADE);
INSERT INTO fkpr VALUES (1.1);
INSERT INTO fkcr VALUES (9, 1.1);
UPDATE fkpr SET x = 2.5 WHERE x = 1.1::real;
-- @test
SELECT id, x::text AS shown FROM fkcc
UNION ALL
SELECT id, x::text FROM fkcr
ORDER BY 1;

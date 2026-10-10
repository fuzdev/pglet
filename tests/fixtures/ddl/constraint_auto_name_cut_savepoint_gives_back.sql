-- A chosen name is claimed by nothing but the catalog: rolled back, it is free again, so
-- the same statements choose the same undigited names a second time.
-- @setup
CREATE TABLE cac7_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo INT, id INT PRIMARY KEY);
SAVEPOINT cac7;
ALTER TABLE cac7_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD UNIQUE (cooooooooooooooooooooooooooooooooooooooo);
ALTER TABLE cac7_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD CHECK (cooooooooooooooooooooooooooooooooooooooo > 0);
ALTER TABLE cac7_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD FOREIGN KEY (cooooooooooooooooooooooooooooooooooooooo) REFERENCES cac7_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt;
ROLLBACK TO SAVEPOINT cac7;
ALTER TABLE cac7_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD UNIQUE (cooooooooooooooooooooooooooooooooooooooo);
ALTER TABLE cac7_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD CHECK (cooooooooooooooooooooooooooooooooooooooo > 0);
ALTER TABLE cac7_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD FOREIGN KEY (cooooooooooooooooooooooooooooooooooooooo) REFERENCES cac7_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt;
-- @test
SELECT contype::text AS contype, conname::text AS conname, length(conname::text) AS len
FROM pg_constraint WHERE conname LIKE 'cac7\_%' AND contype <> 'n' ORDER BY contype, conname;

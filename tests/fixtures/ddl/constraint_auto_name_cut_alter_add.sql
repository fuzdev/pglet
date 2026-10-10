-- The `ALTER TABLE … ADD` routes cut the name they choose: `ADD PRIMARY KEY`, `ADD UNIQUE`
-- over one column and over two (in key order, the reverse of the table's), `ADD CHECK` over
-- one column and over two, and `ADD FOREIGN KEY`. Issued twice, the `UNIQUE` and the
-- `FOREIGN KEY` take the digit rather than fail.
-- @setup
CREATE TABLE cac3_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt (eooooooooo INT, cooooooooooooooooooooooooooooooooooooooo INT, id INT NOT NULL);
ALTER TABLE cac3_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD PRIMARY KEY (id);
ALTER TABLE cac3_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD UNIQUE (cooooooooooooooooooooooooooooooooooooooo);
ALTER TABLE cac3_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD UNIQUE (cooooooooooooooooooooooooooooooooooooooo, eooooooooo);
ALTER TABLE cac3_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD UNIQUE (cooooooooooooooooooooooooooooooooooooooo);
ALTER TABLE cac3_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD CHECK (cooooooooooooooooooooooooooooooooooooooo > 0);
ALTER TABLE cac3_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD CHECK (cooooooooooooooooooooooooooooooooooooooo > eooooooooo);
ALTER TABLE cac3_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD FOREIGN KEY (cooooooooooooooooooooooooooooooooooooooo) REFERENCES cac3_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt;
ALTER TABLE cac3_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD FOREIGN KEY (cooooooooooooooooooooooooooooooooooooooo) REFERENCES cac3_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt;
-- @test
SELECT contype::text AS contype, conname::text AS conname, length(conname::text) AS len
FROM pg_constraint WHERE conname LIKE 'cac3\_%' AND contype <> 'n' ORDER BY contype, conname;

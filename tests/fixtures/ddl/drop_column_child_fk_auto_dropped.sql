-- Dropping the **child** side of an FK auto-drops the constraint (an AUTO edge, no
-- CASCADE): information_schema no longer lists it. pglet kept an inert catalog
-- entry whose column no longer resolved.
-- @setup
CREATE TABLE dcfk_par (id INT PRIMARY KEY);
CREATE TABLE dcfk_ch (cid INT PRIMARY KEY, pid INT REFERENCES dcfk_par(id), note TEXT);

-- @test
ALTER TABLE dcfk_ch DROP COLUMN pid;
SELECT count(*)::int AS fk_rows FROM information_schema.table_constraints
WHERE table_schema = 'public' AND table_name = 'dcfk_ch' AND constraint_type = 'FOREIGN KEY';

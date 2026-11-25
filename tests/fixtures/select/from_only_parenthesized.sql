-- `ONLY (t)` is `ONLY t`, an alias after it included.
-- @setup
CREATE TABLE fop (id INT);
INSERT INTO fop VALUES (1), (2);
-- @test
SELECT x.id FROM ONLY (fop) x ORDER BY 1;

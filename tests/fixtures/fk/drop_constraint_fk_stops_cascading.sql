-- A dropped `ON DELETE CASCADE` / `ON DELETE SET NULL` foreign key no longer acts on a
-- parent delete: the children it would have deleted or nulled stay as they were.
-- @profile-lock
-- @setup
CREATE TABLE dcfc_p (id INT PRIMARY KEY);
CREATE TABLE dcfc_c (
  id INT PRIMARY KEY,
  gone INT REFERENCES dcfc_p(id) ON DELETE CASCADE,
  nulled INT REFERENCES dcfc_p(id) ON DELETE SET NULL
);
INSERT INTO dcfc_p VALUES (1), (2);
INSERT INTO dcfc_c VALUES (10, 1, 1), (20, 2, 2);

-- @test
ALTER TABLE dcfc_c DROP CONSTRAINT dcfc_c_gone_fkey, DROP CONSTRAINT dcfc_c_nulled_fkey;
DELETE FROM dcfc_p WHERE id = 1;
SELECT id, gone, nulled FROM dcfc_c ORDER BY id;

-- `DEFAULT VALUES` carries an `ON CONFLICT` clause (`gram.y`'s `InsertStmt` puts
-- `opt_on_conflict` after `insert_rest`, so every alternative gets it). The proposed
-- row is entirely defaults, conflicts with the seeded one, and the `DO UPDATE` runs.

-- @setup
CREATE TABLE dv_oc (id INT PRIMARY KEY DEFAULT 1, v TEXT DEFAULT 'a');
INSERT INTO dv_oc DEFAULT VALUES;

-- @test
INSERT INTO dv_oc DEFAULT VALUES ON CONFLICT (id) DO UPDATE SET v = 'upd' RETURNING id, v;

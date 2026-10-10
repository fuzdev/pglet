-- The parent-side 23503, and the trap this fixture exists for: the **message** names
-- the parent while `t` names the **child** (`errtableconstraint(fk_rel, …)`), so
-- deriving `t` from the message gets it backwards. The DETAIL's key is the *parent's*
-- referenced tuple.
-- @setup
CREATE TABLE p_fk2 (id INT PRIMARY KEY);
CREATE TABLE c_fk2 (a INT REFERENCES p_fk2(id));
INSERT INTO p_fk2 VALUES (1);
INSERT INTO c_fk2 VALUES (1);

-- @test
-- @expect-error: a child still references this parent
-- @profile-lock
-- @expect-message
DELETE FROM p_fk2 WHERE id = 1;

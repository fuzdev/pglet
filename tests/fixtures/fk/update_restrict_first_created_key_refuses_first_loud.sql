-- The actions on the foreign keys that reference one table run in the order the keys were
-- created: `zc` is created before `ac`, so `zc_k_fkey`'s `ON UPDATE RESTRICT` refuses the
-- move, though `ac` sorts first by name.
-- @setup
CREATE TABLE p (k int PRIMARY KEY);
CREATE TABLE zc (k int REFERENCES p ON UPDATE RESTRICT);
CREATE TABLE ac (k int REFERENCES p ON UPDATE RESTRICT);
INSERT INTO p VALUES (1);
INSERT INTO zc VALUES (1);
INSERT INTO ac VALUES (1);
-- @test
-- @expect-error: the first-created key refuses first
-- @expect-message
-- @profile-lock
UPDATE p SET k = 2;

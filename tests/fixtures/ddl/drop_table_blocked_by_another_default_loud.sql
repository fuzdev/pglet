-- The sequence `dtbad_owner` owns would go with it, and `dtbad_user.x` draws from that
-- sequence from outside the deletion set — a NORMAL edge, so the drop is refused. This
-- refusal is the correctness prerequisite of the auto-drop, not an addition to it:
-- without it the drop leaves a second table's DEFAULT naming a sequence that is gone.
-- @setup
CREATE TABLE dtbad_owner (a SERIAL, b INT);
CREATE TABLE dtbad_user (x INT DEFAULT nextval('dtbad_owner_a_seq'));
-- @test
DROP TABLE dtbad_owner;
-- @expect-error: another table's default depends on the owned sequence (2BP01)

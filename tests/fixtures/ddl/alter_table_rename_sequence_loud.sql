-- `ALTER TABLE <a sequence> RENAME TO` is refused with the wrong kind's `42809` here;
-- 18.7 renames the sequence, so any pglet error passes.
-- @profile-lock
-- @setup
CREATE SEQUENCE rs_seq;
-- @expect-error: cannot open relation "rs_seq"
-- @test
ALTER TABLE rs_seq RENAME TO rs_seq2;

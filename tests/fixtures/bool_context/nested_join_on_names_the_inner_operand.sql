-- Both `ON`s fail the gate, so the SQLSTATE agrees whichever is reached first and this
-- fixture cannot tell them apart on its own — what separates them is the operand named
-- in the message (`integer`, the inner `ON`'s, against `text`, the outer's), which
-- `@expect-message` cannot assert while pglet's `42804` carries a `type error: `
-- prefix. The naming half is pinned by `boolean_context_verdicts.rs`'s
-- `nested_join_on_reports_the_inner_joins_operand`; what this fixture holds is that
-- the code is `42804` on both engines.
-- @setup
CREATE TABLE bcnj10 (k INT, t TEXT);
CREATE TABLE bcnj11 (k INT, t TEXT);
CREATE TABLE bcnj12 (k INT, t TEXT);
-- @expect-error: argument of JOIN/ON must be type boolean, not type integer
-- @test
SELECT 1 FROM bcnj10 JOIN bcnj11 ON bcnj10.k JOIN bcnj12 ON bcnj11.t;

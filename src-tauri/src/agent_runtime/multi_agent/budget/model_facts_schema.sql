-- Original provider cost facts survive task deletion and never grant execution.
CREATE TABLE IF NOT EXISTS agent_model_cost_facts (
 family TEXT NOT NULL CHECK(family IN ('specialist','source-round')),
 root_run_id TEXT NOT NULL,
 assignment_id TEXT NOT NULL,
 lease_attempt_id TEXT NOT NULL,
 child_run_id TEXT NOT NULL,
 lease_epoch INTEGER NOT NULL CHECK(lease_epoch>0),
 fencing_token TEXT NOT NULL,
 round_number INTEGER NOT NULL CHECK(round_number>0),
 request_hash TEXT NOT NULL CHECK(length(request_hash)=64),
 phase TEXT NOT NULL CHECK(phase IN ('received','uncertain','unsent')),
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
 PRIMARY KEY(family,child_run_id,round_number)
);
CREATE TRIGGER IF NOT EXISTS model_cost_fact_no_update BEFORE UPDATE ON agent_model_cost_facts
 BEGIN SELECT RAISE(ABORT,'model_cost_fact_immutable'); END;
CREATE TRIGGER IF NOT EXISTS model_cost_fact_no_delete BEFORE DELETE ON agent_model_cost_facts
 BEGIN SELECT RAISE(ABORT,'model_cost_fact_immutable'); END;
CREATE TRIGGER IF NOT EXISTS model_cost_fact_no_replace BEFORE INSERT ON agent_model_cost_facts
 WHEN EXISTS(SELECT 1 FROM agent_model_cost_facts WHERE family=NEW.family
   AND child_run_id=NEW.child_run_id AND round_number=NEW.round_number)
 BEGIN SELECT RAISE(ABORT,'model_cost_fact_immutable'); END;
CREATE TRIGGER IF NOT EXISTS source_round_no_unsent_transition BEFORE UPDATE ON agent_source_model_rounds
 WHEN EXISTS(SELECT 1 FROM agent_model_cost_facts f WHERE f.family='source-round'
   AND f.root_run_id=OLD.root_run_id AND f.assignment_id=OLD.assignment_id
   AND f.child_run_id=OLD.child_run_id AND f.round_number=OLD.round_number
   AND f.lease_epoch=OLD.lease_epoch AND f.fencing_token=OLD.fencing_token
   AND f.request_hash=OLD.request_hash AND f.phase='unsent')
 BEGIN SELECT RAISE(ABORT,'source_round_unsent_immutable'); END;

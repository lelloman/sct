ALTER TABLE checkpoints ADD COLUMN export_server_epoch bigint CHECK(export_server_epoch>0);

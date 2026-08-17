import json
import os
import socket
import tempfile
import threading
import unittest
from pathlib import Path

from cixa import CixaClient


class ClientTests(unittest.TestCase):
    def test_v1_line_protocol_and_file_token(self):
        with tempfile.TemporaryDirectory() as directory:
            socket_path = os.path.join(directory, "broker.sock")
            token_path = os.path.join(directory, "agent.token")
            Path(token_path).write_text("synthetic-token\n", encoding="utf-8")
            listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            listener.bind(socket_path)
            listener.listen(1)

            def serve():
                channel, _ = listener.accept()
                with channel:
                    request = json.loads(channel.recv(65536).decode("utf-8"))
                    self.assertEqual(request["api_version"], "v1")
                    self.assertEqual(request["token"], "synthetic-token")
                    channel.sendall((json.dumps({"api_version": "v1", "request_id": request["request_id"], "ok": True, "data": {"principal": "agent"}}) + "\n").encode("utf-8"))

            worker = threading.Thread(target=serve)
            worker.start()
            client = CixaClient(socket_path, token_path)
            self.assertEqual(client.get_status(), {"principal": "agent"})
            worker.join(timeout=2)
            listener.close()


if __name__ == "__main__":
    unittest.main()


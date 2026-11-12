"""Push the dataset to Hugging Face:  python dataset/push_dataset.py"""
import os
from huggingface_hub import HfApi, login

login()  # paste a WRITE token from huggingface.co/settings/tokens
api = HfApi()
repo = api.whoami()["name"] + "/paypal-boardroom-ai"
api.create_repo(repo, repo_type="dataset", exist_ok=True)
here = os.path.dirname(os.path.abspath(__file__))
for name in ("paypal_boardroom_dataset.jsonl", "README.md"):
    api.upload_file(path_or_fileobj=os.path.join(here, name), path_in_repo=name,
                    repo_id=repo, repo_type="dataset")
print("Pushed: https://huggingface.co/datasets/" + repo)

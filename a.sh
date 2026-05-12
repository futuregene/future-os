for token in Nqa2dZn8nobG6sxr0dqcNeAenne FNPtdjanRowQKPxBlSBcwLfpnSf TZksdw1kJosvfrxQKdpcAMaon1d GTkcdRbx5o8GTjx6XxJcni4enQe Fm1ad607goEH6exqmfFcjqJznhY NLvCdEkNMo7u66xcMKKcDN4Wnld NRI4d94euoV1NYx9mahcn7nfnqf; do
      echo "=== DOC: $token ==="
      lark-cli docs +fetch --api-version v2 --doc "$token" --as user
    done > /tmp/feishu_ai_docs.json

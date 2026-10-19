FROM public.ecr.aws/lambda/nodejs:24
WORKDIR /var/task
COPY app.js ./app.js
ENTRYPOINT ["/lambda-entrypoint.sh"]
CMD ["app.handler"]
